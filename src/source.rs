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
    if !verify_snapshot(directory, &manifest)? {
        bail!(
            "input snapshot does not match its manifest: {}",
            directory.display()
        );
    }

    let mut files = BTreeMap::new();
    for source in SOURCE_FILES {
        let path = directory.join(source.name);
        let bytes = fs::read(&path)
            .with_context(|| format!("failed to read source file {}", path.display()))?;
        files.insert(source.name.to_owned(), bytes);
    }

    Ok(Snapshot { manifest, files })
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
