use std::cmp::max;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use anyhow::{Context, Result, anyhow, bail};
use ipnet::{IpNet, Ipv4Net, Ipv6Net};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpFamily {
    V4,
    V6,
}

impl IpFamily {
    const fn bits(self) -> u8 {
        match self {
            Self::V4 => 32,
            Self::V6 => 128,
        }
    }

    pub const fn number(self) -> u8 {
        match self {
            Self::V4 => 4,
            Self::V6 => 6,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Range {
    start: u128,
    end: u128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IpSet {
    family: IpFamily,
    ranges: Vec<Range>,
}

impl IpSet {
    pub fn empty(family: IpFamily) -> Self {
        Self {
            family,
            ranges: Vec::new(),
        }
    }

    pub fn universe(family: IpFamily) -> Self {
        let end = match family {
            IpFamily::V4 => u32::MAX as u128,
            IpFamily::V6 => u128::MAX,
        };
        Self {
            family,
            ranges: vec![Range { start: 0, end }],
        }
    }

    pub fn parse_cidrs(input: &str, family: IpFamily, label: &str) -> Result<Self> {
        let mut set = Self::empty(family);

        for (index, original_line) in input.lines().enumerate() {
            let original_line = if index == 0 {
                original_line.trim_start_matches('\u{feff}')
            } else {
                original_line
            };
            let line = original_line.trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('#') || line.starts_with('!') {
                bail!("{label}: line {} is not a plain CIDR: {line:?}", index + 1);
            }

            let network = parse_network(line, family)
                .with_context(|| format!("{label}: invalid line {}", index + 1))?;
            set.add_network(network)?;
        }

        Ok(set)
    }

    pub fn parse_rules(input: &str, family: IpFamily, label: &str) -> Result<Self> {
        let mut set = Self::empty(family);

        for (index, original_line) in input.lines().enumerate() {
            let line = original_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let (remove, cidr) = match line.strip_prefix('!') {
                Some(cidr) => (true, cidr),
                None => (false, line),
            };
            if cidr.is_empty() || cidr.starts_with(char::is_whitespace) {
                bail!("{label}: invalid rule on line {}: {line:?}", index + 1);
            }

            let network = parse_network(cidr, family)
                .with_context(|| format!("{label}: invalid line {}", index + 1))?;
            if remove {
                set.remove_network(network)?;
            } else {
                set.add_network(network)?;
            }
        }

        Ok(set)
    }

    pub fn union(&self, other: &Self) -> Result<Self> {
        self.require_same_family(other)?;
        let mut ranges = self.ranges.clone();
        ranges.extend_from_slice(&other.ranges);
        normalize(&mut ranges);
        Ok(Self {
            family: self.family,
            ranges,
        })
    }

    pub fn subtract(&self, other: &Self) -> Result<Self> {
        self.require_same_family(other)?;
        let mut ranges = Vec::new();
        let mut first_possible = 0;

        for source in &self.ranges {
            while first_possible < other.ranges.len()
                && other.ranges[first_possible].end < source.start
            {
                first_possible += 1;
            }

            let mut cursor = Some(source.start);
            let mut index = first_possible;
            while index < other.ranges.len() && other.ranges[index].start <= source.end {
                let removed = other.ranges[index];
                let Some(current) = cursor else {
                    break;
                };
                if removed.end < current {
                    index += 1;
                    continue;
                }
                if removed.start > current {
                    ranges.push(Range {
                        start: current,
                        end: removed.start - 1,
                    });
                }
                if removed.end >= source.end {
                    cursor = None;
                    break;
                }
                cursor = Some(max(current, removed.end + 1));
                index += 1;
            }
            if let Some(start) = cursor {
                ranges.push(Range {
                    start,
                    end: source.end,
                });
            }
        }

        Ok(Self {
            family: self.family,
            ranges,
        })
    }

    pub fn intersects(&self, other: &Self) -> Result<bool> {
        self.require_same_family(other)?;
        let mut left = 0;
        let mut right = 0;
        while left < self.ranges.len() && right < other.ranges.len() {
            let a = self.ranges[left];
            let b = other.ranges[right];
            if a.end < b.start {
                left += 1;
            } else if b.end < a.start {
                right += 1;
            } else {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn contains(&self, address: IpAddr) -> bool {
        let value = match (self.family, address) {
            (IpFamily::V4, IpAddr::V4(address)) => u32::from(address) as u128,
            (IpFamily::V6, IpAddr::V6(address)) => u128::from(address),
            _ => return false,
        };
        self.ranges
            .binary_search_by(|range| {
                if range.end < value {
                    std::cmp::Ordering::Less
                } else if range.start > value {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }

    pub fn to_text(&self) -> Result<String> {
        let mut output = String::new();
        for range in &self.ranges {
            let mut start = range.start;
            loop {
                let mut host_bits = start.trailing_zeros().min(u32::from(self.family.bits())) as u8;
                while block_end(start, host_bits) > range.end {
                    host_bits -= 1;
                }
                let end = block_end(start, host_bits);
                let prefix = self.family.bits() - host_bits;
                let network = match self.family {
                    IpFamily::V4 => IpNet::V4(
                        Ipv4Net::new(Ipv4Addr::from(start as u32), prefix)
                            .context("failed to create normalized IPv4 prefix")?,
                    ),
                    IpFamily::V6 => IpNet::V6(
                        Ipv6Net::new(Ipv6Addr::from(start), prefix)
                            .context("failed to create normalized IPv6 prefix")?,
                    ),
                };
                output.push_str(&network.to_string());
                output.push('\n');

                if end == range.end {
                    break;
                }
                start = end
                    .checked_add(1)
                    .ok_or_else(|| anyhow!("address range overflow"))?;
            }
        }
        Ok(output)
    }

    pub fn prefix_count(&self) -> Result<usize> {
        Ok(self.to_text()?.lines().count())
    }

    fn add_network(&mut self, network: IpNet) -> Result<()> {
        self.require_network_family(network)?;
        self.ranges.push(network_range(network));
        normalize(&mut self.ranges);
        Ok(())
    }

    fn remove_network(&mut self, network: IpNet) -> Result<()> {
        self.require_network_family(network)?;
        let removal = Self {
            family: self.family,
            ranges: vec![network_range(network)],
        };
        *self = self.subtract(&removal)?;
        Ok(())
    }

    fn require_same_family(&self, other: &Self) -> Result<()> {
        if self.family != other.family {
            bail!("cannot combine IPv4 and IPv6 sets");
        }
        Ok(())
    }

    fn require_network_family(&self, network: IpNet) -> Result<()> {
        let network_family = match network {
            IpNet::V4(_) => IpFamily::V4,
            IpNet::V6(_) => IpFamily::V6,
        };
        let matches = matches!(
            (self.family, network),
            (IpFamily::V4, IpNet::V4(_)) | (IpFamily::V6, IpNet::V6(_))
        );
        if !matches {
            bail!(
                "IPv{} set cannot contain IPv{} prefix",
                self.family.number(),
                network_family.number()
            );
        }
        Ok(())
    }
}

fn parse_network(line: &str, family: IpFamily) -> Result<IpNet> {
    let network: IpNet = line
        .parse()
        .with_context(|| format!("not a CIDR: {line:?}"))?;
    let (actual_family, canonical) = match network {
        IpNet::V4(network) => (IpFamily::V4, network.addr() == network.network()),
        IpNet::V6(network) => (IpFamily::V6, network.addr() == network.network()),
    };
    if actual_family != family {
        bail!(
            "expected IPv{}, found IPv{}",
            family.number(),
            actual_family.number()
        );
    }
    if !canonical {
        bail!("CIDR has host bits set: {line:?}");
    }
    Ok(network)
}

fn network_range(network: IpNet) -> Range {
    match network {
        IpNet::V4(network) => Range {
            start: u32::from(network.network()) as u128,
            end: u32::from(network.broadcast()) as u128,
        },
        IpNet::V6(network) => {
            let start = u128::from(network.network());
            let host_bits = 128 - network.prefix_len();
            Range {
                start,
                end: block_end(start, host_bits),
            }
        }
    }
}

fn block_end(start: u128, host_bits: u8) -> u128 {
    if host_bits == 128 {
        u128::MAX
    } else {
        start + ((1_u128 << host_bits) - 1)
    }
}

fn normalize(ranges: &mut Vec<Range>) {
    ranges.sort_unstable_by_key(|range| range.start);
    let mut normalized: Vec<Range> = Vec::with_capacity(ranges.len());
    for range in ranges.drain(..) {
        if let Some(last) = normalized.last_mut()
            && (range.start <= last.end
                || last
                    .end
                    .checked_add(1)
                    .is_some_and(|next| range.start == next))
        {
            last.end = max(last.end, range.end);
        } else {
            normalized.push(range);
        }
    }
    *ranges = normalized;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_adjacent_prefixes_into_minimal_output() {
        let set = IpSet::parse_cidrs(
            "10.0.0.0/9\n10.128.0.0/9\n192.0.2.0/25\n192.0.2.128/25\n",
            IpFamily::V4,
            "test",
        )
        .unwrap();

        assert_eq!(set.to_text().unwrap(), "10.0.0.0/8\n192.0.2.0/24\n");
    }

    #[test]
    fn applies_ordered_add_and_remove_rules() {
        let set = IpSet::parse_rules(
            "192.0.0.0/24\n!192.0.0.9/32\n!192.0.0.10/32\n",
            IpFamily::V4,
            "test",
        )
        .unwrap();

        assert!(set.contains("192.0.0.8".parse().unwrap()));
        assert!(!set.contains("192.0.0.9".parse().unwrap()));
        assert!(!set.contains("192.0.0.10".parse().unwrap()));
        assert!(set.contains("192.0.0.11".parse().unwrap()));
    }

    #[test]
    fn subtracts_sets_without_crossing_address_families() {
        let removed =
            IpSet::parse_cidrs("10.0.0.0/8\n192.168.0.0/16\n", IpFamily::V4, "test").unwrap();
        let public = IpSet::universe(IpFamily::V4).subtract(&removed).unwrap();

        assert!(!public.contains("10.0.0.1".parse().unwrap()));
        assert!(!public.contains("192.168.1.1".parse().unwrap()));
        assert!(public.contains("8.8.8.8".parse().unwrap()));
        assert!(public.subtract(&IpSet::universe(IpFamily::V6)).is_err());
    }

    #[test]
    fn represents_the_complete_ipv6_space() {
        let set = IpSet::universe(IpFamily::V6);
        assert_eq!(set.to_text().unwrap(), "::/0\n");
    }

    #[test]
    fn rejects_wrong_family_and_host_bits() {
        assert!(IpSet::parse_cidrs("2001:db8::/32\n", IpFamily::V4, "test").is_err());
        assert!(IpSet::parse_cidrs("192.0.2.1/24\n", IpFamily::V4, "test").is_err());
    }
}
