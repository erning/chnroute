use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use tempfile::Builder as TempDirBuilder;

use crate::ipset::{IpFamily, IpSet};
use crate::publish::publish_staged_directory;
use crate::source::{MANIFEST_FILE, REPOSITORY_ID, Snapshot, load_snapshot, sha256};

const OUTPUT_SCHEMA_VERSION: u32 = 2;
const LEGACY_OUTPUT_SCHEMA_VERSION: u32 = 1;
const GENERATOR_ID: &str = "chnroute";
const PRIVATE_V4: &str = include_str!("../data/builtin/private.txt");
const PRIVATE_V6: &str = include_str!("../data/builtin/private6.txt");
const SPECIAL_V4: &str = include_str!("../data/builtin/special.txt");
const SPECIAL_V6: &str = include_str!("../data/builtin/special6.txt");

const OUTPUT_FILES: [&str; 24] = [
    "chnroute.txt",
    "chnroute6.txt",
    "chnroute46.txt",
    "non-chnroute.txt",
    "non-chnroute6.txt",
    "non-chnroute46.txt",
    "china-telecom.txt",
    "china-telecom6.txt",
    "china-telecom46.txt",
    "china-mobile.txt",
    "china-mobile6.txt",
    "china-mobile46.txt",
    "china-unicom.txt",
    "china-unicom6.txt",
    "china-unicom46.txt",
    "china-other.txt",
    "china-other6.txt",
    "china-other46.txt",
    "private.txt",
    "private6.txt",
    "private46.txt",
    "special.txt",
    "special6.txt",
    "special46.txt",
];

#[derive(Debug)]
pub struct GenerateOptions {
    pub input: PathBuf,
    pub output: PathBuf,
}

#[derive(Debug)]
pub struct GenerateResult {
    pub repository: &'static str,
    pub commit: String,
    pub file_count: usize,
    pub prefix_count: usize,
    pub output: PathBuf,
}

struct GeneratedFile {
    name: String,
    address_family: Vec<u8>,
    text: String,
}

struct GeneratedSet {
    base: &'static str,
    set: IpSet,
}

#[derive(Debug, Deserialize, Serialize)]
struct OutputManifest {
    schema_version: u32,
    generator: String,
    source: OutputSource,
    files: BTreeMap<String, OutputFileManifest>,
}

#[derive(Debug, Deserialize, Serialize)]
struct OutputSource {
    repository: String,
    commit: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct OutputFileManifest {
    sha256: String,
    bytes: u64,
    prefixes: usize,
    #[serde(deserialize_with = "deserialize_address_family")]
    address_family: Vec<u8>,
}

fn deserialize_address_family<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum AddressFamily {
        Legacy(u8),
        Current(Vec<u8>),
    }

    match AddressFamily::deserialize(deserializer)? {
        AddressFamily::Legacy(family) => Ok(vec![family]),
        AddressFamily::Current(families) => Ok(families),
    }
}

pub fn generate(options: GenerateOptions) -> Result<GenerateResult> {
    validate_options(&options)?;
    let snapshot = load_snapshot(&options.input)?;

    let parent = directory_parent(&options.output);
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create output parent: {}", parent.display()))?;
    reject_overlapping_directories(&options.input, &options.output)?;
    inspect_output(&options.output)?;

    let generated = build_files(&snapshot)?;
    let output_name = options
        .output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("output directory must have a valid final component"))?;
    let staging = TempDirBuilder::new()
        .prefix(&format!(".{output_name}.generate-"))
        .tempdir_in(parent)
        .with_context(|| format!("failed to create staging directory in {}", parent.display()))?;

    let mut files = BTreeMap::new();
    for file in generated {
        let bytes = file.text.as_bytes();
        write_staged_file(staging.path(), &file.name, bytes)?;
        files.insert(
            file.name,
            OutputFileManifest {
                sha256: sha256(bytes),
                bytes: bytes.len() as u64,
                prefixes: file.text.lines().count(),
                address_family: file.address_family,
            },
        );
    }

    let manifest = OutputManifest {
        schema_version: OUTPUT_SCHEMA_VERSION,
        generator: GENERATOR_ID.to_owned(),
        source: OutputSource {
            repository: snapshot.manifest.source.repository,
            commit: snapshot.manifest.source.commit.clone(),
        },
        files,
    };
    write_output_manifest(staging.path(), &manifest)?;

    let prefix_count = manifest.files.values().map(|file| file.prefixes).sum();
    let file_count = manifest.files.len();
    let commit = manifest.source.commit.clone();
    publish_staged_directory(staging, &options.output)?;

    Ok(GenerateResult {
        repository: REPOSITORY_ID,
        commit,
        file_count,
        prefix_count,
        output: options.output,
    })
}

fn validate_options(options: &GenerateOptions) -> Result<()> {
    if options.input.as_os_str().is_empty() {
        bail!("--input must name a fetched snapshot directory");
    }
    if options.output.as_os_str().is_empty()
        || options.output == Path::new(".")
        || options.output == Path::new("..")
        || options.output.file_name().is_none()
    {
        bail!("--output must name a dedicated distribution directory");
    }
    Ok(())
}

fn reject_overlapping_directories(input: &Path, output: &Path) -> Result<()> {
    let input = fs::canonicalize(input)
        .with_context(|| format!("failed to resolve input: {}", input.display()))?;
    let output = if output.exists() {
        fs::canonicalize(output)
            .with_context(|| format!("failed to resolve output: {}", output.display()))?
    } else {
        let parent = directory_parent(output);
        let parent = fs::canonicalize(parent)
            .with_context(|| format!("failed to resolve output parent: {}", parent.display()))?;
        let name = output
            .file_name()
            .ok_or_else(|| anyhow!("output directory must have a final component"))?;
        parent.join(name)
    };

    if input.starts_with(&output) || output.starts_with(&input) {
        bail!("input and output directories must not overlap");
    }
    Ok(())
}

fn directory_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn inspect_output(output: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(output) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect output: {}", output.display()));
        }
    };

    if metadata.file_type().is_symlink() {
        bail!("refusing to replace symlink output: {}", output.display());
    }
    if !metadata.is_dir() {
        bail!("output exists but is not a directory: {}", output.display());
    }

    let manifest_path = output.join(MANIFEST_FILE);
    if !manifest_path.exists() {
        if fs::read_dir(output)
            .with_context(|| format!("failed to read output: {}", output.display()))?
            .next()
            .is_none()
        {
            return Ok(());
        }
        bail!(
            "refusing to replace unowned directory without {}: {}",
            MANIFEST_FILE,
            output.display()
        );
    }

    let bytes = fs::read(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let manifest: OutputManifest = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    if !matches!(
        manifest.schema_version,
        LEGACY_OUTPUT_SCHEMA_VERSION | OUTPUT_SCHEMA_VERSION
    ) || manifest.generator != GENERATOR_ID
    {
        bail!(
            "refusing to replace output owned by an incompatible generator or schema: {}",
            output.display()
        );
    }

    Ok(())
}

fn build_files(snapshot: &Snapshot) -> Result<Vec<GeneratedFile>> {
    let ipv4 = build_family(snapshot, IpFamily::V4, PRIVATE_V4, SPECIAL_V4, "")?;
    let ipv6 = build_family(snapshot, IpFamily::V6, PRIVATE_V6, SPECIAL_V6, "6")?;

    if ipv4.len() != ipv6.len() {
        bail!("internal IPv4 and IPv6 output mappings do not match");
    }

    let mut files = Vec::with_capacity(OUTPUT_FILES.len());
    for (ipv4, ipv6) in ipv4.into_iter().zip(ipv6) {
        if ipv4.base != ipv6.base {
            bail!("internal IPv4 and IPv6 output mappings do not match");
        }

        let ipv4_text = ipv4.set.to_text()?;
        let ipv6_text = ipv6.set.to_text()?;
        let mut combined_text = String::with_capacity(ipv4_text.len() + ipv6_text.len());
        combined_text.push_str(&ipv4_text);
        combined_text.push_str(&ipv6_text);

        files.push(generated(ipv4.base, "", vec![4], ipv4_text));
        files.push(generated(ipv4.base, "6", vec![6], ipv6_text));
        files.push(generated(ipv4.base, "46", vec![4, 6], combined_text));
    }

    if files.len() != OUTPUT_FILES.len()
        || OUTPUT_FILES
            .iter()
            .any(|expected| !files.iter().any(|file| file.name == *expected))
    {
        bail!("internal output file mapping is incomplete");
    }
    Ok(files)
}

fn build_family(
    snapshot: &Snapshot,
    family: IpFamily,
    private_rules: &str,
    special_rules: &str,
    suffix: &str,
) -> Result<Vec<GeneratedSet>> {
    // Output semantics:
    // chnroute     = public addresses in mainland China
    // non-chnroute = public addresses outside chnroute
    // china-other  = other mainland China operators
    let private = IpSet::parse_rules(private_rules, family, "built-in private rules")?;
    let special = IpSet::parse_rules(special_rules, family, "built-in special rules")?;
    if private.intersects(&special)? {
        bail!(
            "built-in private and special sets overlap for IPv{}",
            family.number()
        );
    }

    let china = source_set(snapshot, &format!("china{suffix}.txt"), family)?;
    let telecom = source_set(snapshot, &format!("chinanet{suffix}.txt"), family)?;
    let mobile = source_set(snapshot, &format!("cmcc{suffix}.txt"), family)?;
    let unicom = source_set(snapshot, &format!("unicom{suffix}.txt"), family)?;

    let mut other = IpSet::empty(family);
    for source in ["cernet", "cstnet", "drpeng", "googlecn"] {
        let set = source_set(snapshot, &format!("{source}{suffix}.txt"), family)?;
        other = other.union(&set)?;
    }

    let non_chnroute = IpSet::universe(family)
        .subtract(&private)?
        .subtract(&special)?
        .subtract(&china)?;

    Ok(vec![
        generated_set("chnroute", china),
        generated_set("non-chnroute", non_chnroute),
        generated_set("china-telecom", telecom),
        generated_set("china-mobile", mobile),
        generated_set("china-unicom", unicom),
        generated_set("china-other", other),
        generated_set("private", private),
        generated_set("special", special),
    ])
}

fn source_set(snapshot: &Snapshot, name: &str, family: IpFamily) -> Result<IpSet> {
    let bytes = snapshot
        .files
        .get(name)
        .ok_or_else(|| anyhow!("verified input snapshot is missing {name}"))?;
    let text = std::str::from_utf8(bytes).with_context(|| format!("{name} is not valid UTF-8"))?;
    IpSet::parse_cidrs(text, family, name)
}

fn generated_set(base: &'static str, set: IpSet) -> GeneratedSet {
    GeneratedSet { base, set }
}

fn generated(base: &str, suffix: &str, address_family: Vec<u8>, text: String) -> GeneratedFile {
    GeneratedFile {
        name: format!("{base}{suffix}.txt"),
        address_family,
        text,
    }
}

fn write_staged_file(staging: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let path = staging.join(name);
    let mut file = File::create(&path)
        .with_context(|| format!("failed to create staged file {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("failed to write staged file {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync staged file {}", path.display()))?;
    Ok(())
}

fn write_output_manifest(staging: &Path, manifest: &OutputManifest) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(manifest).context("failed to serialize manifest")?;
    bytes.push(b'\n');
    write_staged_file(staging, MANIFEST_FILE, &bytes)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::source::{
        AddressFamily, FileManifest, MANIFEST_SCHEMA_VERSION, Manifest, SOURCE_FILES,
        SourceManifest,
    };

    const COMMIT: &str = "1111111111111111111111111111111111111111";

    #[test]
    fn generates_exact_canonical_files_and_set_membership() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("raw");
        let output = temp.path().join("dist");
        write_source_snapshot(&input);

        let result = generate(GenerateOptions {
            input,
            output: output.clone(),
        })
        .unwrap();

        assert_eq!(result.file_count, 24);
        let names: BTreeSet<String> = fs::read_dir(&output)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        let mut expected: BTreeSet<String> =
            OUTPUT_FILES.iter().map(|name| name.to_string()).collect();
        expected.insert(MANIFEST_FILE.to_owned());
        assert_eq!(names, expected);

        assert_eq!(
            fs::read_to_string(output.join("chnroute.txt")).unwrap(),
            "1.0.0.0/24\n"
        );
        assert_eq!(
            fs::read_to_string(output.join("china-other.txt")).unwrap(),
            "1.0.1.0/24\n"
        );
        assert_eq!(
            fs::read_to_string(output.join("china-other6.txt")).unwrap(),
            "2401::/32\n"
        );
        assert_eq!(
            fs::read_to_string(output.join("chnroute46.txt")).unwrap(),
            "1.0.0.0/24\n2400::/32\n"
        );
        assert_eq!(
            fs::read_to_string(output.join("china-other46.txt")).unwrap(),
            "1.0.1.0/24\n2401::/32\n"
        );

        for combined_name in OUTPUT_FILES.iter().filter(|name| name.ends_with("46.txt")) {
            let base = combined_name.strip_suffix("46.txt").unwrap();
            let expected = format!(
                "{}{}",
                fs::read_to_string(output.join(format!("{base}.txt"))).unwrap(),
                fs::read_to_string(output.join(format!("{base}6.txt"))).unwrap()
            );
            assert_eq!(
                fs::read_to_string(output.join(combined_name)).unwrap(),
                expected
            );
        }

        let manifest: OutputManifest =
            serde_json::from_slice(&fs::read(output.join(MANIFEST_FILE)).unwrap()).unwrap();
        for name in OUTPUT_FILES {
            let expected_family = if name.ends_with("46.txt") {
                vec![4, 6]
            } else if name.ends_with("6.txt") {
                vec![6]
            } else {
                vec![4]
            };
            assert_eq!(manifest.files[name].address_family, expected_family);
        }

        let non_v4 = IpSet::parse_cidrs(
            &fs::read_to_string(output.join("non-chnroute.txt")).unwrap(),
            IpFamily::V4,
            "non-chnroute.txt",
        )
        .unwrap();
        assert!(non_v4.contains("8.8.8.8".parse().unwrap()));
        assert!(!non_v4.contains("1.0.0.1".parse().unwrap()));
        assert!(!non_v4.contains("10.0.0.1".parse().unwrap()));
        assert!(!non_v4.contains("127.0.0.1".parse().unwrap()));

        let non_v6 = IpSet::parse_cidrs(
            &fs::read_to_string(output.join("non-chnroute6.txt")).unwrap(),
            IpFamily::V6,
            "non-chnroute6.txt",
        )
        .unwrap();
        assert!(non_v6.contains("2001:4860::1".parse().unwrap()));
        assert!(!non_v6.contains("2400::1".parse().unwrap()));
        assert!(!non_v6.contains("fc00::1".parse().unwrap()));
        assert!(!non_v6.contains("2001:db8::1".parse().unwrap()));
    }

    #[test]
    fn rejects_a_source_file_that_does_not_match_the_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("raw");
        let output = temp.path().join("dist");
        write_source_snapshot(&input);
        fs::write(input.join("china.txt"), b"8.8.8.0/24\n").unwrap();

        assert!(generate(GenerateOptions { input, output }).is_err());
    }

    #[test]
    fn refuses_to_replace_an_unowned_output_directory() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("raw");
        let output = temp.path().join("dist");
        write_source_snapshot(&input);
        fs::create_dir(&output).unwrap();
        fs::write(output.join("user-file.txt"), b"keep me").unwrap();

        assert!(
            generate(GenerateOptions {
                input,
                output: output.clone(),
            })
            .is_err()
        );
        assert_eq!(fs::read(output.join("user-file.txt")).unwrap(), b"keep me");
    }

    #[test]
    fn can_replace_its_own_previous_output() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("raw");
        let output = temp.path().join("dist");
        write_source_snapshot(&input);

        generate(GenerateOptions {
            input: input.clone(),
            output: output.clone(),
        })
        .unwrap();
        generate(GenerateOptions { input, output }).unwrap();
    }

    #[test]
    fn can_replace_legacy_schema_one_output() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("raw");
        let output = temp.path().join("dist");
        write_source_snapshot(&input);

        generate(GenerateOptions {
            input: input.clone(),
            output: output.clone(),
        })
        .unwrap();

        let manifest_path = output.join(MANIFEST_FILE);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["schema_version"] = serde_json::json!(1);
        let files = manifest["files"].as_object_mut().unwrap();
        files.retain(|name, _| !name.ends_with("46.txt"));
        for file in files.values_mut() {
            let family = file["address_family"].as_array().unwrap()[0].clone();
            file["address_family"] = family;
        }
        let mut bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        bytes.push(b'\n');
        fs::write(&manifest_path, bytes).unwrap();
        for name in OUTPUT_FILES.iter().filter(|name| name.ends_with("46.txt")) {
            fs::remove_file(output.join(name)).unwrap();
        }

        generate(GenerateOptions {
            input,
            output: output.clone(),
        })
        .unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(output.join(MANIFEST_FILE)).unwrap()).unwrap();
        assert_eq!(manifest["schema_version"], serde_json::json!(2));
        assert_eq!(
            manifest["files"]["chnroute46.txt"]["address_family"],
            serde_json::json!([4, 6])
        );
    }

    fn write_source_snapshot(directory: &Path) {
        fs::create_dir(directory).unwrap();
        let mut files = BTreeMap::new();

        for source in SOURCE_FILES {
            let text = match source.name {
                "china.txt" => "1.0.0.0/24\n",
                "china6.txt" => "2400::/32\n",
                "chinanet.txt" => "1.0.0.0/25\n",
                "cmcc.txt" => "1.0.0.128/26\n",
                "unicom.txt" => "1.0.0.192/26\n",
                "cernet.txt" => "1.0.1.0/25\n",
                "cstnet.txt" => "1.0.1.128/25\n",
                "cernet6.txt" => "2401::/33\n",
                "cstnet6.txt" => "2401:0:8000::/33\n",
                _ => "",
            };
            let bytes = text.as_bytes();
            fs::write(directory.join(source.name), bytes).unwrap();
            files.insert(
                source.name.to_owned(),
                FileManifest {
                    sha256: sha256(bytes),
                    bytes: bytes.len() as u64,
                    prefixes: text.lines().count(),
                    address_family: match source.family {
                        AddressFamily::Ipv4 => 4,
                        AddressFamily::Ipv6 => 6,
                    },
                },
            );
        }

        let manifest = Manifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            source: SourceManifest {
                repository: REPOSITORY_ID.to_owned(),
                requested_ref: "ip-lists".to_owned(),
                commit: COMMIT.to_owned(),
            },
            fetched_at: "2026-08-17T00:00:00Z".to_owned(),
            files,
        };
        let mut bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        bytes.push(b'\n');
        fs::write(directory.join(MANIFEST_FILE), bytes).unwrap();
    }
}
