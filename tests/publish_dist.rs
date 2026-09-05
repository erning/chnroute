#![cfg(unix)]

use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    repository: PathBuf,
    remote: PathBuf,
    path: OsString,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repository = temp.path().join("source");
        let remote = temp.path().join("remote.git");
        let bin = temp.path().join("bin");
        fs::create_dir_all(repository.join("scripts")).unwrap();
        fs::create_dir(&bin).unwrap();
        fs::write(
            repository.join("scripts/publish-dist.sh"),
            include_str!("../scripts/publish-dist.sh"),
        )
        .unwrap();
        fs::write(repository.join("Cargo.toml"), "# Script test fixture\n").unwrap();
        fs::write(repository.join(".gitignore"), "/dist/\n").unwrap();

        // Exercise the publisher with fixed artifacts and real local Git
        // repositories, without invoking Cargo recursively or using a network.
        let cargo = bin.join("cargo");
        fs::write(
            &cargo,
            concat!(
                "#!/bin/sh\nset -eu\n",
                "case \"$*\" in\n",
                "  'test --locked --offline') exit 0 ;;\n",
                "  'run --locked --offline --release -- generate')\n",
                "    mkdir -p dist\n",
                "    printf '{}\\n' > dist/manifest.json\n",
                "    printf '1.0.0.0/24\\n' > dist/chnroute.txt ;;\n",
                "  *) exit 1 ;;\n",
                "esac\n",
            ),
        )
        .unwrap();
        fs::set_permissions(cargo, fs::Permissions::from_mode(0o755)).unwrap();
        let mut paths = vec![bin];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        let fixture = Self {
            _temp: temp,
            repository,
            remote,
            path: std::env::join_paths(paths).unwrap(),
        };
        fixture.git(&["init", "--quiet", "--initial-branch=master"]);
        fixture.git(&["config", "user.name", "Publisher Test"]);
        fixture.git(&["config", "user.email", "publisher@example.invalid"]);
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "--quiet", "-m", "source fixture"]);
        fixture.git(&[
            "init",
            "--quiet",
            "--bare",
            fixture.remote.to_str().unwrap(),
        ]);
        fixture.git(&["remote", "add", "origin", fixture.remote.to_str().unwrap()]);
        fixture
    }

    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command
            .current_dir(&self.repository)
            .env("PATH", &self.path)
            .env("LC_ALL", "C")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_COMMON_DIR");
        command
    }

    fn git(&self, args: &[&str]) -> String {
        let output = self.command("git").args(args).output().unwrap();
        assert_success(&output);
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn remote_git(&self, args: &[&str]) -> String {
        let mut command = self.command("git");
        command.arg("--git-dir").arg(&self.remote).args(args);
        let output = command.output().unwrap();
        assert_success(&output);
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn publish(&self, push: bool) -> Output {
        let mut command = self.command("sh");
        command.arg("scripts/publish-dist.sh");
        if push {
            command.arg("--push");
        }
        command.output().unwrap()
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn pushes_an_existing_local_commit_without_creating_another_commit() {
    let fixture = Fixture::new();
    let source_commit = fixture.git(&["rev-parse", "HEAD"]);
    assert_success(&fixture.publish(false));
    let dist_commit = fixture.git(&["rev-parse", "refs/heads/dist"]);
    assert_eq!(fixture.remote_git(&["for-each-ref", "refs/heads/dist"]), "");

    for _ in 0..2 {
        assert_success(&fixture.publish(true));
        assert_eq!(
            fixture.remote_git(&["rev-parse", "refs/heads/dist"]),
            dist_commit
        );
        assert_eq!(fixture.git(&["rev-parse", "refs/heads/dist"]), dist_commit);
    }
    assert_eq!(
        fixture.git(&["rev-list", "--count", "refs/heads/dist"]),
        "1"
    );
    assert_eq!(fixture.git(&["rev-parse", "HEAD"]), source_commit);
    assert_eq!(fixture.git(&["branch", "--show-current"]), "master");
    assert_eq!(fixture.git(&["status", "--porcelain"]), "");
}

#[test]
fn retries_a_failed_push_when_artifacts_are_unchanged() {
    let fixture = Fixture::new();
    let unavailable = fixture.remote.with_file_name("unavailable.git");
    fixture.git(&["remote", "set-url", "origin", unavailable.to_str().unwrap()]);

    assert!(!fixture.publish(true).status.success());
    let dist_commit = fixture.git(&["rev-parse", "refs/heads/dist"]);
    fixture.git(&[
        "remote",
        "set-url",
        "origin",
        fixture.remote.to_str().unwrap(),
    ]);

    assert_success(&fixture.publish(true));
    assert_eq!(
        fixture.remote_git(&["rev-parse", "refs/heads/dist"]),
        dist_commit
    );
    assert_eq!(fixture.git(&["rev-parse", "refs/heads/dist"]), dist_commit);
}

#[test]
fn refuses_to_overwrite_an_advanced_remote_when_artifacts_are_unchanged() {
    let fixture = Fixture::new();
    assert_success(&fixture.publish(true));
    let local_commit = fixture.git(&["rev-parse", "refs/heads/dist"]);
    let remote_commit = fixture.remote_git(&[
        "-c",
        "user.name=Publisher Test",
        "-c",
        "user.email=publisher@example.invalid",
        "commit-tree",
        "refs/heads/dist^{tree}",
        "-p",
        &local_commit,
        "-m",
        "remote update",
    ]);
    fixture.remote_git(&["update-ref", "refs/heads/dist", &remote_commit]);

    assert!(!fixture.publish(true).status.success());
    assert_eq!(
        fixture.remote_git(&["rev-parse", "refs/heads/dist"]),
        remote_commit
    );
    assert_eq!(fixture.git(&["rev-parse", "refs/heads/dist"]), local_commit);
}
