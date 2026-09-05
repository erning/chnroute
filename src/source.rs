use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const REPOSITORY_ID: &str = "gaoyifan/china-operator-ip";
pub(crate) const MANIFEST_FILE: &str = "manifest.json";
pub(crate) const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AddressFamily {
    Ipv4,
    Ipv6,
}

impl AddressFamily {
    pub(crate) const fn number(self) -> u8 {
        match self {
            Self::Ipv4 => 4,
            Self::Ipv6 => 6,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SourceFile {
    pub(crate) name: &'static str,
    pub(crate) family: AddressFamily,
}

pub(crate) const SOURCE_FILES: [SourceFile; 16] = [
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

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Manifest {
    pub(crate) schema_version: u32,
    pub(crate) source: SourceManifest,
    pub(crate) fetched_at: String,
    pub(crate) files: BTreeMap<String, FileManifest>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct SourceManifest {
    pub(crate) repository: String,
    pub(crate) requested_ref: String,
    pub(crate) commit: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct FileManifest {
    pub(crate) sha256: String,
    pub(crate) bytes: u64,
    pub(crate) prefixes: usize,
    pub(crate) address_family: u8,
}

pub(crate) struct Snapshot {
    pub(crate) manifest: Manifest,
    pub(crate) files: BTreeMap<String, Vec<u8>>,
}

pub(crate) fn read_manifest(directory: &Path) -> Result<Manifest> {
    let path = directory.join(MANIFEST_FILE);
    let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION
        || manifest.source.repository != REPOSITORY_ID
    {
        bail!(
            "incompatible source or manifest schema in {}",
            path.display()
        );
    }
    Ok(manifest)
}

pub(crate) fn verify_snapshot(directory: &Path, manifest: &Manifest) -> Result<bool> {
    visit_verified_files(directory, manifest, |_, _| {})
}

fn visit_verified_files(
    directory: &Path,
    manifest: &Manifest,
    mut visit: impl FnMut(&str, Vec<u8>),
) -> Result<bool> {
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
        let path = directory.join(source.name);
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
        visit(source.name, bytes);
    }

    Ok(true)
}

pub(crate) fn load_snapshot(directory: &Path) -> Result<Snapshot> {
    let metadata = fs::symlink_metadata(directory)
        .with_context(|| format!("failed to inspect input: {}", directory.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("input must not be a symlink: {}", directory.display());
    }
    if !metadata.is_dir() {
        bail!("input is not a directory: {}", directory.display());
    }

    let manifest = read_manifest(directory)?;
    let mut files = BTreeMap::new();
    if !visit_verified_files(directory, &manifest, |name, bytes| {
        files.insert(name.to_owned(), bytes);
    })? {
        bail!(
            "input snapshot does not match its manifest: {}",
            directory.display()
        );
    }

    Ok(Snapshot { manifest, files })
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_snapshot(directory: &Path) -> Manifest {
        let mut files = BTreeMap::new();
        for source in SOURCE_FILES {
            let bytes: &[u8] = if source.name == "china.txt" {
                b"1.0.0.0/24\n"
            } else {
                b""
            };
            fs::write(directory.join(source.name), bytes).unwrap();
            files.insert(
                source.name.to_owned(),
                FileManifest {
                    sha256: sha256(bytes),
                    bytes: bytes.len() as u64,
                    prefixes: usize::from(!bytes.is_empty()),
                    address_family: source.family.number(),
                },
            );
        }
        let manifest = Manifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            source: SourceManifest {
                repository: REPOSITORY_ID.to_owned(),
                requested_ref: "ip-lists".to_owned(),
                commit: "1".repeat(40),
            },
            fetched_at: "2026-09-06T00:00:00Z".to_owned(),
            files,
        };
        fs::write(
            directory.join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        manifest
    }

    #[test]
    fn loads_a_complete_snapshot_including_empty_sets() {
        let temp = tempfile::tempdir().unwrap();
        write_snapshot(temp.path());

        let snapshot = load_snapshot(temp.path()).unwrap();

        assert_eq!(snapshot.files.len(), SOURCE_FILES.len());
        assert_eq!(snapshot.files["china.txt"], b"1.0.0.0/24\n");
        assert!(snapshot.files["china6.txt"].is_empty());
        for (name, bytes) in &snapshot.files {
            assert_eq!(sha256(bytes), snapshot.manifest.files[name].sha256);
            assert_eq!(bytes.len() as u64, snapshot.manifest.files[name].bytes);
        }
    }

    #[test]
    fn retains_verified_bytes_when_an_already_read_file_changes() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = write_snapshot(temp.path());
        let mut files = BTreeMap::new();

        let verified = visit_verified_files(temp.path(), &manifest, |name, bytes| {
            files.insert(name.to_owned(), bytes);
            if name == "googlecn6.txt" {
                fs::write(temp.path().join("china.txt"), b"8.8.8.0/24\n").unwrap();
            }
        })
        .unwrap();

        assert!(verified);
        assert_eq!(files["china.txt"], b"1.0.0.0/24\n");
        assert_eq!(
            sha256(&files["china.txt"]),
            manifest.files["china.txt"].sha256
        );
        assert!(!verify_snapshot(temp.path(), &manifest).unwrap());
        assert!(load_snapshot(temp.path()).is_err());
    }

    #[test]
    fn rejects_missing_files_and_invalid_manifest_entries() {
        for failure in ["missing file", "missing entry", "family", "size", "hash"] {
            let temp = tempfile::tempdir().unwrap();
            let mut manifest = write_snapshot(temp.path());
            match failure {
                "missing file" => fs::remove_file(temp.path().join("china.txt")).unwrap(),
                "missing entry" => {
                    manifest.files.remove("china.txt");
                }
                "family" => manifest.files.get_mut("china.txt").unwrap().address_family = 6,
                "size" => manifest.files.get_mut("china.txt").unwrap().bytes += 1,
                "hash" => manifest.files.get_mut("china.txt").unwrap().sha256 = "0".repeat(64),
                _ => unreachable!(),
            }
            fs::write(
                temp.path().join(MANIFEST_FILE),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();

            assert!(
                !verify_snapshot(temp.path(), &manifest).unwrap(),
                "{failure}"
            );
            assert!(load_snapshot(temp.path()).is_err(), "{failure}");
        }
    }
}
