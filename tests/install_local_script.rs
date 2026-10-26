//! `scripts/install-local.sh --from-ci` installs the binary CI built for a commit
//! instead of compiling it, and builds locally only when CI published nothing for
//! that commit (hivemind-b3my9).
//!
//! `.github/workflows/edge.yml` publishes `loomtracer-linux-x86_64-<sha12>.tar.gz`
//! and its `.sha256` on the `edge` release. Here `curl` serves such a release from
//! a directory (a real 200, 404 or other status), `uname` says Linux x86_64 and
//! `cargo` writes a stand-in binary; the script itself, `git`, `tar`, `install`
//! and the checksum tool run for real.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, MutexGuard, PoisonError};

use sha2::{Digest, Sha256};
use tempfile::TempDir;

type TestResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const FAKE_UNAME: &str = r#"#!/bin/sh
case "$1" in
  -m) echo "${SHIM_UNAME_M:-x86_64}" ;;
  *) echo "${SHIM_UNAME_S:-Linux}" ;;
esac
"#;

/// Answers a GET under `releases/download/edge/` from `$SHIM_EDGE_DIR` the way
/// `curl -sSL -o FILE -w '%{http_code}'` would: 200 and the file, 404 when there is none,
/// or `$SHIM_EDGE_STATUS` for every file when that is set. Each URL is logged to `$SHIM_LOG`.
const FAKE_CURL: &str = r#"#!/bin/sh
out=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift ;;
    -w) shift ;;
    -*) ;;
    *) url="$1" ;;
  esac
  shift
done
echo "curl $url" >> "$SHIM_LOG"
file="$SHIM_EDGE_DIR/${url##*/}"
if [ -n "${SHIM_EDGE_STATUS:-}" ]; then
  printf '%s' "$SHIM_EDGE_STATUS"
elif [ -f "$file" ]; then
  cp "$file" "$out"
  printf 200
else
  printf 404
fi
"#;

/// Stands in for the compile: leaves a binary where `cargo build --release` would.
const FAKE_CARGO: &str = r#"#!/bin/sh
echo "cargo $*" >> "$SHIM_LOG"
mkdir -p "$CARGO_TARGET_DIR/release"
printf '#!/bin/sh\necho "loomtracer 0.0.0+built-here"\n' > "$CARGO_TARGET_DIR/release/loomtracer"
chmod 755 "$CARGO_TARGET_DIR/release/loomtracer"
"#;

/// The tests write executables and then run them; a `fork` from a sibling test
/// thread in between can hold the write descriptor open and fail the `exec`
/// with ETXTBSY. Running the tests one at a time rules that out.
static SERIAL: Mutex<()> = Mutex::new(());

#[derive(Default)]
struct Run {
    flags: Vec<&'static str>,
    /// What `uname -s` and `uname -m` answer, when not Linux x86_64.
    uname: Option<(&'static str, &'static str)>,
    /// The status the release server answers every download with.
    edge_status: Option<&'static str>,
}

struct Host {
    _serial: MutexGuard<'static, ()>,
    root: TempDir,
    short_sha: String,
}

impl Host {
    fn new() -> TestResult<Self> {
        let serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
        let root = TempDir::new()?;
        let host = Self {
            _serial: serial,
            short_sha: String::new(),
            root,
        };
        for dir in [host.repo(), host.bin(), host.edge_dir(), host.install_dir()] {
            fs::create_dir(dir)?;
        }
        for (name, body) in [
            ("uname", FAKE_UNAME),
            ("curl", FAKE_CURL),
            ("cargo", FAKE_CARGO),
        ] {
            let path = host.bin().join(name);
            fs::write(&path, body)?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        }
        git(&host.repo(), &["init", "-q"])?;
        git(
            &host.repo(),
            &[
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ],
        )?;
        let short_sha = git(&host.repo(), &["rev-parse", "--short=12", "HEAD"])?;
        Ok(Self { short_sha, ..host })
    }

    /// The checkout the script runs from.
    fn repo(&self) -> PathBuf {
        self.root.path().join("repo")
    }

    fn bin(&self) -> PathBuf {
        self.root.path().join("bin")
    }

    /// What the `edge` release holds, as the fake `curl` serves it.
    fn edge_dir(&self) -> PathBuf {
        self.root.path().join("edge")
    }

    fn install_dir(&self) -> PathBuf {
        self.root.path().join("installed")
    }

    fn installed(&self) -> PathBuf {
        self.install_dir().join("loomtracer")
    }

    fn log(&self) -> PathBuf {
        self.root.path().join("shim.log")
    }

    fn asset(&self) -> String {
        format!("loomtracer-linux-x86_64-{}.tar.gz", self.short_sha)
    }

    /// CI published a binary that reports `version`, with its checksum.
    fn publish_binary(&self, version: &str) -> TestResult<()> {
        let staging = self.root.path().join("staging");
        fs::create_dir_all(&staging)?;
        let binary = staging.join("loomtracer");
        fs::write(
            &binary,
            format!("#!/bin/sh\necho \"loomtracer {version}\"\n"),
        )?;
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))?;
        let tarball = self.edge_dir().join(self.asset());
        let status = Command::new("tar")
            .arg("-C")
            .arg(&staging)
            .arg("-czf")
            .arg(&tarball)
            .arg("loomtracer")
            .status()?;
        assert!(status.success());
        self.publish_checksum(&hex(&fs::read(&tarball)?))
    }

    /// `sha256sum`'s line for the tarball, with `hex_digest` as its digest.
    fn publish_checksum(&self, hex_digest: &str) -> TestResult<()> {
        fs::write(
            self.edge_dir().join(format!("{}.sha256", self.asset())),
            format!("{hex_digest}  {}\n", self.asset()),
        )?;
        Ok(())
    }

    /// A binary already installed, as an earlier deploy left it.
    fn install_old_binary(&self) -> TestResult<String> {
        let old = "#!/bin/sh\necho \"loomtracer 0.0.0+old\"\n";
        fs::write(self.installed(), old)?;
        fs::set_permissions(self.installed(), fs::Permissions::from_mode(0o755))?;
        Ok(old.to_owned())
    }

    fn run(&self, run: Run) -> TestResult<Output> {
        let path = std::env::join_paths(std::iter::once(self.bin()).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )))?;
        let (uname_s, uname_m) = run.uname.unwrap_or(("Linux", "x86_64"));
        let mut command = Command::new("sh");
        command
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/install-local.sh"))
            .args(&run.flags)
            .arg("HEAD")
            .current_dir(self.repo())
            .env("PATH", path)
            .env("SHIM_LOG", self.log())
            .env("SHIM_EDGE_DIR", self.edge_dir())
            .env("SHIM_UNAME_S", uname_s)
            .env("SHIM_UNAME_M", uname_m)
            .env("LOOMTRACER_INSTALL_DIR", self.install_dir())
            .env(
                "LOOMTRACER_BUILD_TARGET_DIR",
                self.root.path().join("target"),
            )
            .env_remove("SHIM_EDGE_STATUS")
            .env_remove("LOOMTRACER_REPO");
        if let Some(status) = run.edge_status {
            command.env("SHIM_EDGE_STATUS", status);
        }
        Ok(command.output()?)
    }

    fn run_from_ci(&self, flags: &[&'static str]) -> TestResult<Output> {
        let mut all = vec!["--from-ci"];
        all.extend_from_slice(flags);
        self.run(Run {
            flags: all,
            ..Run::default()
        })
    }

    /// What the stand-in `curl` and `cargo` were asked, in order.
    fn calls(&self) -> TestResult<Vec<String>> {
        if !self.log().exists() {
            return Ok(Vec::new());
        }
        Ok(fs::read_to_string(self.log())?
            .lines()
            .map(str::to_owned)
            .collect())
    }

    fn cargo_ran(&self) -> TestResult<bool> {
        Ok(self.calls()?.iter().any(|call| call.starts_with("cargo ")))
    }

    /// What the installed binary reports.
    fn installed_version(&self) -> TestResult<String> {
        let output = Command::new(self.installed()).arg("--version").output()?;
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }
}

fn git(dir: &Path, args: &[&str]) -> TestResult<String> {
    let output = Command::new("git").args(args).current_dir(dir).output()?;
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn from_ci_installs_the_ci_built_binary_without_compiling() -> TestResult<()> {
    let host = Host::new()?;
    host.publish_binary(&format!("0.0.0+{}", host.short_sha))?;

    let output = host.run_from_ci(&[])?;
    assert!(output.status.success(), "{}", stderr(&output));

    assert_eq!(
        host.installed_version()?,
        format!("loomtracer 0.0.0+{}", host.short_sha)
    );
    assert_eq!(
        fs::metadata(host.installed())?.permissions().mode() & 0o777,
        0o755
    );
    assert!(!host.cargo_ran()?, "{:?}", host.calls()?);
    // It asked for the commit's own assets, the checksum first.
    let base = "https://github.com/alexknips/loomtracer/releases/download/edge";
    assert_eq!(
        host.calls()?,
        vec![
            format!("curl {base}/{}.sha256", host.asset()),
            format!("curl {base}/{}", host.asset()),
        ]
    );
    assert!(stdout(&output).contains("CI-built"), "{}", stdout(&output));
    Ok(())
}

#[test]
fn from_ci_builds_locally_when_ci_published_no_binary_for_the_commit() -> TestResult<()> {
    let host = Host::new()?;

    let output = host.run_from_ci(&[])?;
    assert!(output.status.success(), "{}", stderr(&output));

    assert!(host.cargo_ran()?);
    assert_eq!(host.installed_version()?, "loomtracer 0.0.0+built-here");
    assert!(
        stdout(&output).contains("CI has no binary"),
        "{}",
        stdout(&output)
    );
    Ok(())
}

#[test]
fn from_ci_no_build_exits_3_and_installs_nothing_when_ci_has_no_binary() -> TestResult<()> {
    let host = Host::new()?;
    let old = host.install_old_binary()?;

    let output = host.run_from_ci(&["--no-build"])?;

    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("not building"),
        "{}",
        stderr(&output)
    );
    assert!(!host.cargo_ran()?);
    assert_eq!(fs::read_to_string(host.installed())?, old);
    Ok(())
}

#[test]
fn a_checksum_that_does_not_match_stops_the_install_and_leaves_the_old_binary() -> TestResult<()> {
    let host = Host::new()?;
    host.publish_binary(&format!("0.0.0+{}", host.short_sha))?;
    host.publish_checksum(&"0".repeat(64))?;
    let old = host.install_old_binary()?;

    let output = host.run_from_ci(&[])?;

    assert!(!output.status.success());
    // Corruption is an incident, not a reason to quietly compile something else.
    assert!(!host.cargo_ran()?);
    assert_eq!(fs::read_to_string(host.installed())?, old);
    Ok(())
}

#[test]
fn a_binary_that_reports_another_commit_is_not_installed() -> TestResult<()> {
    let host = Host::new()?;
    host.publish_binary("0.0.0+ffffffffffff")?;
    let old = host.install_old_binary()?;

    let output = host.run_from_ci(&[])?;

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains(&format!("not {}", host.short_sha)),
        "{}",
        stderr(&output)
    );
    assert!(!host.cargo_ran()?);
    assert_eq!(fs::read_to_string(host.installed())?, old);
    Ok(())
}

#[test]
fn a_release_server_that_fails_does_not_start_a_local_build() -> TestResult<()> {
    let host = Host::new()?;
    host.publish_binary(&format!("0.0.0+{}", host.short_sha))?;

    let output = host.run(Run {
        flags: vec!["--from-ci"],
        edge_status: Some("503"),
        ..Run::default()
    })?;

    // Only a 404 means "CI has not published this".
    assert!(!output.status.success());
    assert!(stderr(&output).contains("HTTP 503"), "{}", stderr(&output));
    assert!(!host.cargo_ran()?);
    assert!(!host.installed().exists());
    Ok(())
}

#[test]
fn a_published_checksum_without_its_tarball_stops_the_install() -> TestResult<()> {
    let host = Host::new()?;
    host.publish_checksum(&"0".repeat(64))?;

    let output = host.run_from_ci(&[])?;

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("could not be fetched"),
        "{}",
        stderr(&output)
    );
    assert!(!host.cargo_ran()?);
    Ok(())
}

#[test]
fn from_ci_on_a_platform_ci_does_not_build_falls_back_to_building() -> TestResult<()> {
    let host = Host::new()?;
    host.publish_binary(&format!("0.0.0+{}", host.short_sha))?;

    let output = host.run(Run {
        flags: vec!["--from-ci"],
        uname: Some(("Darwin", "arm64")),
        ..Run::default()
    })?;
    assert!(output.status.success(), "{}", stderr(&output));

    assert!(host.cargo_ran()?);
    assert!(
        stderr(&output).contains("Linux x86_64 only"),
        "{}",
        stderr(&output)
    );
    // It never went to the server for a binary that cannot run here.
    assert!(host.calls()?.iter().all(|call| !call.starts_with("curl ")));
    Ok(())
}

#[test]
fn without_from_ci_the_script_builds_and_never_asks_the_release_server() -> TestResult<()> {
    let host = Host::new()?;
    host.publish_binary(&format!("0.0.0+{}", host.short_sha))?;

    let output = host.run(Run::default())?;
    assert!(output.status.success(), "{}", stderr(&output));

    assert!(host.cargo_ran()?);
    assert!(host.calls()?.iter().all(|call| !call.starts_with("curl ")));
    assert_eq!(host.installed_version()?, "loomtracer 0.0.0+built-here");
    Ok(())
}

#[test]
fn no_build_without_from_ci_is_a_usage_error() -> TestResult<()> {
    let host = Host::new()?;

    let output = host.run(Run {
        flags: vec!["--no-build"],
        ..Run::default()
    })?;

    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(!host.cargo_ran()?);
    assert!(!host.installed().exists());
    Ok(())
}
