use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use ipnet::IpNet;
use reqwest::Url;
use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::{Builder as TempDirBuilder, TempDir};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const OWNER: &str = "gaoyifan";
const REPOSITORY: &str = "china-operator-ip";
const REPOSITORY_ID: &str = "gaoyifan/china-operator-ip";
const GITHUB_ROOT: &str = "https://github.com/";
const RAW_ROOT: &str = "https://raw.githubusercontent.com/";
const USER_AGENT_VALUE: &str = "chnroute/0.1";
const MANIFEST_FILE: &str = "manifest.json";
const MANIFEST_SCHEMA_VERSION: u32 = 1;
const MAX_REF_DISCOVERY_BYTES: usize = 4 * 1024 * 1024;
const MAX_DATA_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AddressFamily {
    Ipv4,
    Ipv6,
}

impl AddressFamily {
    const fn number(self) -> u8 {
        match self {
            Self::Ipv4 => 4,
            Self::Ipv6 => 6,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct SourceFile {
    name: &'static str,
    family: AddressFamily,
}

const SOURCE_FILES: [SourceFile; 16] = [
    SourceFile {
        name: "china.txt",
        family: AddressFamily::Ipv4,
    },
    SourceFile {
        name: "china6.txt",
        family: AddressFamily::Ipv6,
    },
    SourceFile {
        name: "chinanet.txt",
        family: AddressFamily::Ipv4,
    },
    SourceFile {
        name: "chinanet6.txt",
        family: AddressFamily::Ipv6,
    },
    SourceFile {
        name: "cmcc.txt",
        family: AddressFamily::Ipv4,
    },
    SourceFile {
        name: "cmcc6.txt",
        family: AddressFamily::Ipv6,
    },
    SourceFile {
        name: "unicom.txt",
        family: AddressFamily::Ipv4,
    },
    SourceFile {
        name: "unicom6.txt",
        family: AddressFamily::Ipv6,
    },
    SourceFile {
        name: "cernet.txt",
        family: AddressFamily::Ipv4,
    },
    SourceFile {
        name: "cernet6.txt",
        family: AddressFamily::Ipv6,
    },
    SourceFile {
        name: "cstnet.txt",
        family: AddressFamily::Ipv4,
    },
    SourceFile {
        name: "cstnet6.txt",
        family: AddressFamily::Ipv6,
    },
    SourceFile {
        name: "drpeng.txt",
        family: AddressFamily::Ipv4,
    },
    SourceFile {
        name: "drpeng6.txt",
        family: AddressFamily::Ipv6,
    },
    SourceFile {
        name: "googlecn.txt",
        family: AddressFamily::Ipv4,
    },
    SourceFile {
        name: "googlecn6.txt",
        family: AddressFamily::Ipv6,
    },
];

#[derive(Debug)]
pub struct FetchOptions {
    pub requested_ref: String,
    pub output: PathBuf,
    pub force: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FetchStatus {
    Updated,
    Unchanged,
}

#[derive(Debug)]
pub struct FetchResult {
    pub status: FetchStatus,
    pub repository: &'static str,
    pub commit: String,
    pub file_count: usize,
    pub prefix_count: usize,
}

#[derive(Debug, Deserialize, Serialize)]
struct Manifest {
    schema_version: u32,
    source: SourceManifest,
    fetched_at: String,
    files: BTreeMap<String, FileManifest>,
}

#[derive(Debug, Deserialize, Serialize)]
struct SourceManifest {
    repository: String,
    requested_ref: String,
    commit: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct FileManifest {
    sha256: String,
    bytes: u64,
    prefixes: usize,
    address_family: u8,
}

trait HttpClient {
    fn get(&self, url: Url, accept: &'static str, max_bytes: usize) -> Result<Vec<u8>>;
}

struct GitHubClient {
    client: Client,
}

impl GitHubClient {
    fn from_environment() -> Result<Self> {
        let client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::limited(5))
            .timeout(Duration::from_secs(30))
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self { client })
    }
}

impl HttpClient for GitHubClient {
    fn get(&self, url: Url, accept: &'static str, max_bytes: usize) -> Result<Vec<u8>> {
        let request = self
            .client
            .get(url.clone())
            .header(USER_AGENT, USER_AGENT_VALUE)
            .header(ACCEPT, accept);

        let response = request
            .send()
            .with_context(|| format!("request failed: {url}"))?;

        read_response(response, &url, max_bytes)
    }
}

pub fn fetch(options: FetchOptions) -> Result<FetchResult> {
    let client = GitHubClient::from_environment()?;
    fetch_with_client(&client, options)
}

fn fetch_with_client(client: &dyn HttpClient, options: FetchOptions) -> Result<FetchResult> {
    validate_options(&options)?;
    let commit = resolve_commit(client, &options.requested_ref)?;
    let existing_manifest = inspect_output(&options.output)?;

    if !options.force
        && let Some(manifest) = existing_manifest.as_ref()
        && manifest.source.requested_ref == options.requested_ref
        && manifest.source.commit == commit
        && verify_snapshot(&options.output, manifest)?
    {
        return Ok(result_from_manifest(
            FetchStatus::Unchanged,
            commit,
            manifest,
        ));
    }

    let parent = options.output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create output parent: {}", parent.display()))?;
    let output_name = options
        .output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("output directory must have a valid final component"))?;
    let staging = TempDirBuilder::new()
        .prefix(&format!(".{output_name}.fetch-"))
        .tempdir_in(parent)
        .with_context(|| format!("failed to create staging directory in {}", parent.display()))?;

    let mut files = BTreeMap::new();
    for source in SOURCE_FILES {
        let url = raw_file_url(&commit, source.name)?;
        let bytes = client
            .get(url, "application/octet-stream", MAX_DATA_BYTES)
            .with_context(|| format!("failed to download {}", source.name))?;
        let prefixes = validate_cidrs(source, &bytes)
            .with_context(|| format!("invalid upstream file {}", source.name))?;
        write_staged_file(staging.path(), source.name, &bytes)?;

        files.insert(
            source.name.to_owned(),
            FileManifest {
                sha256: sha256(&bytes),
                bytes: bytes.len() as u64,
                prefixes,
                address_family: source.family.number(),
            },
        );
    }

    let fetched_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .context("failed to format fetch timestamp")?;
    let manifest = Manifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        source: SourceManifest {
            repository: REPOSITORY_ID.to_owned(),
            requested_ref: options.requested_ref,
            commit: commit.clone(),
        },
        fetched_at,
        files,
    };
    write_manifest(staging.path(), &manifest)?;
    publish(staging, &options.output)?;

    Ok(result_from_manifest(
        FetchStatus::Updated,
        commit,
        &manifest,
    ))
}

fn validate_options(options: &FetchOptions) -> Result<()> {
    if options.requested_ref.trim().is_empty() {
        bail!("--ref must not be empty");
    }
    if options.output.as_os_str().is_empty()
        || options.output == Path::new(".")
        || options.output == Path::new("..")
        || options.output.file_name().is_none()
    {
        bail!("--output must name a dedicated data directory");
    }
    Ok(())
}

fn inspect_output(output: &Path) -> Result<Option<Manifest>> {
    let metadata = match fs::symlink_metadata(output) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
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
            return Ok(None);
        }
        bail!(
            "refusing to replace unowned directory without {}: {}",
            MANIFEST_FILE,
            output.display()
        );
    }

    let bytes = fs::read(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION
        || manifest.source.repository != REPOSITORY_ID
    {
        bail!(
            "refusing to replace output owned by an incompatible source or schema: {}",
            output.display()
        );
    }

    Ok(Some(manifest))
}

fn resolve_commit(client: &dyn HttpClient, requested_ref: &str) -> Result<String> {
    if is_commit_sha(requested_ref) {
        return Ok(requested_ref.to_ascii_lowercase());
    }

    let url = git_ref_url()?;
    let bytes = client
        .get(
            url,
            "application/x-git-upload-pack-advertisement",
            MAX_REF_DISCOVERY_BYTES,
        )
        .with_context(|| format!("failed to resolve upstream ref {requested_ref:?}"))?;
    parse_git_advertisement(&bytes, requested_ref)
}

fn git_ref_url() -> Result<Url> {
    let mut url = Url::parse(GITHUB_ROOT).context("invalid built-in GitHub URL")?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("GitHub URL cannot accept path segments"))?
        .extend([OWNER, &format!("{REPOSITORY}.git"), "info", "refs"]);
    url.query_pairs_mut()
        .append_pair("service", "git-upload-pack");
    Ok(url)
}

fn parse_git_advertisement(bytes: &[u8], requested_ref: &str) -> Result<String> {
    let branch_name = format!("refs/heads/{requested_ref}");
    let tag_name = format!("refs/tags/{requested_ref}");
    let peeled_tag_name = format!("{tag_name}^{{}}");
    let mut branch = None;
    let mut tag = None;
    let mut peeled_tag = None;
    let mut cursor = 0;

    while cursor < bytes.len() {
        if bytes.len() - cursor < 4 {
            bail!("truncated Git reference advertisement");
        }
        let length_text = std::str::from_utf8(&bytes[cursor..cursor + 4])
            .context("Git packet length is not UTF-8")?;
        let packet_length = usize::from_str_radix(length_text, 16)
            .with_context(|| format!("invalid Git packet length {length_text:?}"))?;
        cursor += 4;

        if packet_length <= 2 {
            continue;
        }
        if packet_length < 4 || cursor + packet_length - 4 > bytes.len() {
            bail!("invalid Git packet length {packet_length}");
        }

        let payload = &bytes[cursor..cursor + packet_length - 4];
        cursor += packet_length - 4;
        let payload = std::str::from_utf8(payload).context("Git packet is not valid UTF-8")?;
        let payload = payload.trim_end_matches('\n');
        if payload.starts_with("# service=") {
            continue;
        }

        let Some((commit, reference)) = payload.split_once(' ') else {
            continue;
        };
        if !is_commit_sha(commit) {
            bail!("Git returned an invalid commit SHA: {commit:?}");
        }
        let reference = reference.split('\0').next().unwrap_or(reference);
        let commit = commit.to_ascii_lowercase();
        if reference == branch_name {
            branch = Some(commit);
        } else if reference == peeled_tag_name {
            peeled_tag = Some(commit);
        } else if reference == tag_name {
            tag = Some(commit);
        }
    }

    branch.or(peeled_tag).or(tag).ok_or_else(|| {
        anyhow!(
            "upstream ref {requested_ref:?} is not a branch or tag; use a full 40-character commit SHA"
        )
    })
}

fn is_commit_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn raw_file_url(commit: &str, name: &str) -> Result<Url> {
    let mut url = Url::parse(RAW_ROOT).context("invalid built-in GitHub raw URL")?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("GitHub raw URL cannot accept path segments"))?
        .extend([OWNER, REPOSITORY, commit, name]);
    Ok(url)
}

fn read_response(response: Response, url: &Url, max_bytes: usize) -> Result<Vec<u8>> {
    let status = response.status();
    if !status.is_success() {
        let mut body = String::new();
        response
            .take(8 * 1024)
            .read_to_string(&mut body)
            .context("failed to read HTTP error response")?;
        bail!("HTTP {status} from {url}: {}", body.trim());
    }
    if response
        .content_length()
        .is_some_and(|size| size > max_bytes as u64)
    {
        bail!("response from {url} exceeds the {max_bytes}-byte limit");
    }

    let mut bytes = Vec::new();
    response
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read response from {url}"))?;
    if bytes.len() > max_bytes {
        bail!("response from {url} exceeds the {max_bytes}-byte limit");
    }
    Ok(bytes)
}

fn validate_cidrs(source: SourceFile, bytes: &[u8]) -> Result<usize> {
    let text = std::str::from_utf8(bytes).context("file is not valid UTF-8")?;
    let mut prefixes = 0;

    for (index, original_line) in text.lines().enumerate() {
        let original_line = if index == 0 {
            original_line.trim_start_matches('\u{feff}')
        } else {
            original_line
        };
        let line = original_line.trim();
        if line.is_empty() {
            continue;
        }

        let network: IpNet = line
            .parse()
            .with_context(|| format!("line {} is not a CIDR: {line:?}", index + 1))?;
        let (family, canonical) = match network {
            IpNet::V4(network) => (AddressFamily::Ipv4, network.addr() == network.network()),
            IpNet::V6(network) => (AddressFamily::Ipv6, network.addr() == network.network()),
        };
        if family != source.family {
            bail!(
                "line {} contains IPv{} in an IPv{} file",
                index + 1,
                family.number(),
                source.family.number()
            );
        }
        if !canonical {
            bail!("line {} has host bits set: {line:?}", index + 1);
        }
        prefixes += 1;
    }

    Ok(prefixes)
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

fn write_manifest(staging: &Path, manifest: &Manifest) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(manifest).context("failed to serialize manifest")?;
    bytes.push(b'\n');
    write_staged_file(staging, MANIFEST_FILE, &bytes)
}

fn publish(staging: TempDir, output: &Path) -> Result<()> {
    let staging_path = staging.keep();

    if !output.exists() {
        return fs::rename(&staging_path, output).with_context(|| {
            format!(
                "failed to publish {} to {}",
                staging_path.display(),
                output.display()
            )
        });
    }

    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let output_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("output directory must have a valid final component"))?;
    let backup_reservation = TempDirBuilder::new()
        .prefix(&format!(".{output_name}.backup-"))
        .tempdir_in(parent)
        .context("failed to reserve rollback directory")?;
    let backup = backup_reservation.path().to_owned();
    backup_reservation
        .close()
        .context("failed to prepare rollback directory")?;

    fs::rename(output, &backup).with_context(|| {
        format!(
            "failed to move existing snapshot {} to {}",
            output.display(),
            backup.display()
        )
    })?;

    if let Err(publish_error) = fs::rename(&staging_path, output) {
        let restore_result = fs::rename(&backup, output);
        let _ = fs::remove_dir_all(&staging_path);
        return match restore_result {
            Ok(()) => Err(publish_error).with_context(|| {
                format!(
                    "failed to publish new snapshot; restored {}",
                    output.display()
                )
            }),
            Err(restore_error) => Err(anyhow!(
                "failed to publish new snapshot ({publish_error}) and failed to restore {} ({restore_error}); backup remains at {}",
                output.display(),
                backup.display()
            )),
        };
    }

    let _ = fs::remove_dir_all(backup);
    Ok(())
}

fn verify_snapshot(output: &Path, manifest: &Manifest) -> Result<bool> {
    if manifest.files.len() != SOURCE_FILES.len() {
        return Ok(false);
    }

    for source in SOURCE_FILES {
        let Some(expected) = manifest.files.get(source.name) else {
            return Ok(false);
        };
        if expected.address_family != source.family.number() {
            return Ok(false);
        }
        let path = output.join(source.name);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(error).with_context(|| format!("failed to verify {}", path.display()));
            }
        };
        if bytes.len() as u64 != expected.bytes || sha256(&bytes) != expected.sha256 {
            return Ok(false);
        }
    }

    Ok(true)
}

fn result_from_manifest(status: FetchStatus, commit: String, manifest: &Manifest) -> FetchResult {
    FetchResult {
        status,
        repository: REPOSITORY_ID,
        commit,
        file_count: manifest.files.len(),
        prefix_count: manifest.files.values().map(|file| file.prefixes).sum(),
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    const FIRST_COMMIT: &str = "1111111111111111111111111111111111111111";
    const SECOND_COMMIT: &str = "2222222222222222222222222222222222222222";

    struct FakeClient {
        responses: HashMap<String, Vec<u8>>,
        requests: Mutex<Vec<String>>,
    }

    impl FakeClient {
        fn snapshot(commit: &str, ipv4: &[u8], ipv6: &[u8]) -> Self {
            let mut responses = HashMap::new();
            responses.insert(
                git_ref_url().unwrap().to_string(),
                advertisement(&[(commit, "refs/heads/ip-lists")]),
            );
            for source in SOURCE_FILES {
                responses.insert(
                    raw_file_url(commit, source.name).unwrap().to_string(),
                    match source.family {
                        AddressFamily::Ipv4 => ipv4.to_vec(),
                        AddressFamily::Ipv6 => ipv6.to_vec(),
                    },
                );
            }
            Self {
                responses,
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl HttpClient for FakeClient {
        fn get(&self, url: Url, _accept: &'static str, _max_bytes: usize) -> Result<Vec<u8>> {
            let url = url.to_string();
            self.requests.lock().unwrap().push(url.clone());
            self.responses
                .get(&url)
                .cloned()
                .ok_or_else(|| anyhow!("no fake response for {url}"))
        }
    }

    fn advertisement(refs: &[(&str, &str)]) -> Vec<u8> {
        let mut bytes = packet(b"# service=git-upload-pack\n");
        bytes.extend_from_slice(b"0000");
        for (commit, reference) in refs {
            bytes.extend(packet(format!("{commit} {reference}\n").as_bytes()));
        }
        bytes.extend_from_slice(b"0000");
        bytes
    }

    fn packet(payload: &[u8]) -> Vec<u8> {
        let mut packet = format!("{:04x}", payload.len() + 4).into_bytes();
        packet.extend_from_slice(payload);
        packet
    }

    fn options(output: PathBuf) -> FetchOptions {
        FetchOptions {
            requested_ref: "ip-lists".to_owned(),
            output,
            force: false,
        }
    }

    #[test]
    fn validates_address_family_and_canonical_prefixes() {
        let v4 = SourceFile {
            name: "test.txt",
            family: AddressFamily::Ipv4,
        };
        assert_eq!(validate_cidrs(v4, b"1.0.0.0/24\n10.0.0.0/8\n").unwrap(), 2);
        assert!(validate_cidrs(v4, b"2001:db8::/32\n").is_err());
        assert!(validate_cidrs(v4, b"1.0.0.1/24\n").is_err());
        assert_eq!(validate_cidrs(v4, b"\n").unwrap(), 0);
    }

    #[test]
    fn downloads_exact_files_and_writes_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("raw");
        let client = FakeClient::snapshot(FIRST_COMMIT, b"1.0.0.0/24\n", b"2001:db8::/32\n");

        let result = fetch_with_client(&client, options(output.clone())).unwrap();

        assert_eq!(result.status, FetchStatus::Updated);
        assert_eq!(result.file_count, 16);
        assert_eq!(result.prefix_count, 16);
        assert_eq!(fs::read(output.join("china.txt")).unwrap(), b"1.0.0.0/24\n");
        assert_eq!(
            fs::read(output.join("china6.txt")).unwrap(),
            b"2001:db8::/32\n"
        );
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(output.join(MANIFEST_FILE)).unwrap()).unwrap();
        assert_eq!(manifest.source.commit, FIRST_COMMIT);
        assert_eq!(manifest.files.len(), 16);
    }

    #[test]
    fn skips_valid_current_snapshot_without_downloading_files() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("raw");
        let first = FakeClient::snapshot(FIRST_COMMIT, b"1.0.0.0/24\n", b"2001:db8::/32\n");
        fetch_with_client(&first, options(output.clone())).unwrap();

        let mut second = FakeClient::snapshot(FIRST_COMMIT, b"9.0.0.0/24\n", b"2001:db9::/32\n");
        second.responses.retain(|url, _| url.contains("/info/refs"));
        let result = fetch_with_client(&second, options(output)).unwrap();

        assert_eq!(result.status, FetchStatus::Unchanged);
        assert_eq!(second.requests().len(), 1);
    }

    #[test]
    fn failed_update_keeps_previous_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("raw");
        let first = FakeClient::snapshot(FIRST_COMMIT, b"1.0.0.0/24\n", b"2001:db8::/32\n");
        fetch_with_client(&first, options(output.clone())).unwrap();
        let original_manifest = fs::read(output.join(MANIFEST_FILE)).unwrap();

        let mut second = FakeClient::snapshot(SECOND_COMMIT, b"2.0.0.0/24\n", b"2001:db9::/32\n");
        second.responses.remove(
            &raw_file_url(SECOND_COMMIT, "googlecn6.txt")
                .unwrap()
                .to_string(),
        );
        assert!(fetch_with_client(&second, options(output.clone())).is_err());

        assert_eq!(
            fs::read(output.join(MANIFEST_FILE)).unwrap(),
            original_manifest
        );
        assert_eq!(fs::read(output.join("china.txt")).unwrap(), b"1.0.0.0/24\n");
    }

    #[test]
    fn refuses_to_replace_unowned_directory() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("raw");
        fs::create_dir(&output).unwrap();
        fs::write(output.join("user-file.txt"), b"keep me").unwrap();
        let client = FakeClient::snapshot(FIRST_COMMIT, b"1.0.0.0/24\n", b"2001:db8::/32\n");

        assert!(fetch_with_client(&client, options(output.clone())).is_err());
        assert_eq!(fs::read(output.join("user-file.txt")).unwrap(), b"keep me");
    }

    #[test]
    fn resolves_branches_and_peeled_tags_from_git_advertisement() {
        let bytes = advertisement(&[
            (FIRST_COMMIT, "refs/heads/ip-lists"),
            (FIRST_COMMIT, "refs/tags/v1"),
            (SECOND_COMMIT, "refs/tags/v1^{}"),
        ]);

        assert_eq!(
            parse_git_advertisement(&bytes, "ip-lists").unwrap(),
            FIRST_COMMIT
        );
        assert_eq!(
            parse_git_advertisement(&bytes, "v1").unwrap(),
            SECOND_COMMIT
        );
        assert!(parse_git_advertisement(&bytes, "missing").is_err());
    }
}
