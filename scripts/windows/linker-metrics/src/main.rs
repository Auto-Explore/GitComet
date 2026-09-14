#![recursion_limit = "256"]

mod monitor;
mod response;

use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize)]
pub struct RunSpec {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub cwd: PathBuf,
    pub output_dir: PathBuf,
    #[serde(default = "default_sample")]
    pub sample_ms: u32,
    #[serde(default = "default_timeout")]
    pub timeout_s: u64,
    #[serde(default)]
    pub metadata: Value,
}
fn default_sample() -> u32 {
    50
}
fn default_timeout() -> u64 {
    7200
}

#[derive(Deserialize)]
struct ProxyConfig {
    backend: String,
    executable: PathBuf,
    log_root: PathBuf,
    #[serde(default)]
    capture_dir: Option<PathBuf>,
    #[serde(default = "default_sample")]
    sample_ms: u32,
    #[serde(default)]
    metadata: Value,
}

pub fn save_json(path: &Path, value: &Value) -> Result<()> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    fs::write(path, text)?;
    Ok(())
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn run() -> Result<u32> {
    let wrapper_start = Instant::now();
    let arguments: Vec<String> = env::args_os()
        .skip(1)
        .map(|x| x.into_string().map_err(|_| "non-Unicode command argument"))
        .collect::<std::result::Result<_, _>>()?;
    if arguments.first().is_some_and(|s| s == "--echo-args") {
        println!("{}", serde_json::to_string(&arguments[1..])?);
        return Ok(0);
    }
    if arguments.first().is_some_and(|s| s == "--exit-code") {
        return Ok(arguments.get(1).ok_or("missing exit code")?.parse()?);
    }
    if arguments.first().is_some_and(|s| s == "--sleep-ms") {
        std::thread::sleep(std::time::Duration::from_millis(
            arguments.get(1).ok_or("missing sleep duration")?.parse()?,
        ));
        return Ok(0);
    }
    if arguments.first().is_some_and(|s| s == "--run-spec") {
        if arguments.len() != 2 {
            return Err("usage: gitcomet-linker-metrics --run-spec FILE.json".into());
        }
        let spec: RunSpec = serde_json::from_slice(&fs::read(&arguments[1])?)?;
        return monitor::run(&spec, wrapper_start);
    }
    if arguments.first().is_some_and(|s| s == "--parse-response") {
        let path = arguments.get(1).ok_or("missing response path")?;
        println!(
            "{}",
            serde_json::to_string(&response::read(Path::new(path))?)?
        );
        return Ok(0);
    }
    // A workload used by integration tests: allocate/touch memory, perform I/O,
    // and optionally launch a child so lifetime accounting can be verified.
    if arguments
        .first()
        .is_some_and(|s| s == "--self-test-workload")
    {
        let mut memory = vec![0u8; 32 * 1024 * 1024];
        for (i, byte) in memory.iter_mut().enumerate() {
            *byte = i as u8;
        }
        let path = env::current_dir()?.join("workload.bin");
        fs::write(&path, &memory)?;
        let read = fs::read(&path)?;
        assert_eq!(read.len(), memory.len());
        if arguments.get(1).is_some_and(|s| s == "child") {
            let status = std::process::Command::new(env::current_exe()?)
                .arg("--self-test-workload")
                .status()?;
            if !status.success() {
                return Err("workload child failed".into());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(180));
        println!("workload complete {}", std::hint::black_box(memory[100]));
        return Ok(0);
    }
    let config_path = env::var_os("GITCOMET_LINK_BENCH_CONFIG")
        .ok_or("set GITCOMET_LINK_BENCH_CONFIG or use --run-spec FILE.json")?;
    let config: ProxyConfig = serde_json::from_slice(&fs::read(config_path)?)?;
    if !matches!(config.backend.as_str(), "lld" | "msvc") {
        return Err("invalid proxy backend".into());
    }
    let id = format!(
        "{}-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
        std::process::id()
    );
    let output_dir = config.log_root.join(&id);
    fs::create_dir_all(&output_dir)?;
    let cwd = env::current_dir()?;
    let expanded = response::expand(&arguments, &cwd, &output_dir)?;
    let output = expanded.iter().find_map(|a| {
        a.get(..5)
            .filter(|p| p.eq_ignore_ascii_case("/out:"))
            .map(|_| a[5..].to_owned())
    });
    let is_application = output
        .as_ref()
        .and_then(|p| Path::new(p).file_stem())
        .and_then(|s| s.to_str())
        .is_some_and(|s| s == "gitcomet" || s.starts_with("gitcomet-"));
    let mut native_arguments = Vec::new();
    if config.backend == "lld" {
        native_arguments.extend(["-flavor".into(), "link".into()]);
    }
    native_arguments.push("/STACK:8388608".into());
    native_arguments.extend(arguments.clone());
    native_arguments.push("/INCREMENTAL:NO".into());
    if is_application && let Some(capture_dir) = &config.capture_dir {
        if config.backend != "lld" {
            return Err("fixture capture requires LLD".into());
        }
        fs::create_dir_all(capture_dir)?;
        let archive = capture_dir.join("repro.tar");
        if archive.exists() {
            return Err("capture archive already exists; use a new capture directory".into());
        }
        native_arguments.push(format!("/reproduce:{}", archive.display()));
        save_json(
            &capture_dir.join("original-command.json"),
            &json!({
                "cwd": cwd, "arguments": arguments, "expanded_arguments": expanded,
                "output": output, "proxy_run_id": id,
            }),
        )?;
    }
    let mut metadata = config.metadata;
    if !metadata.is_object() {
        metadata = json!({});
    }
    metadata["linker"] = json!(config.backend);
    metadata["scope"] = json!("linker");
    metadata["invocation_id"] = json!(id);
    metadata["application_link"] = json!(is_application);
    metadata["output"] = json!(output);
    metadata["capture"] = json!(is_application && config.capture_dir.is_some());
    let spec = RunSpec {
        executable: config.executable,
        arguments: native_arguments,
        cwd,
        output_dir: output_dir.clone(),
        sample_ms: config.sample_ms,
        timeout_s: default_timeout(),
        metadata,
    };
    let exit = monitor::run(&spec, wrapper_start)?;
    // Keep compiler-visible diagnostics. Regular files avoid pipe backpressure;
    // the raw logs remain available even if the compiler suppresses success logs.
    io::copy(
        &mut fs::File::open(output_dir.join("stdout.log"))?,
        &mut io::stdout(),
    )?;
    io::copy(
        &mut fs::File::open(output_dir.join("stderr.log"))?,
        &mut io::stderr(),
    )?;
    Ok(exit)
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code as i32),
        Err(error) => {
            eprintln!("linker-metrics: {error}");
            std::process::exit(125);
        }
    }
}
