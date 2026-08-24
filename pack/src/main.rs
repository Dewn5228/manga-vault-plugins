use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::Digest;
use serde::Deserialize;

const WASM_TARGET: &str = "wasm32-wasip2";

#[derive(Deserialize)]
struct Manifest {
	id: String,
	backend: Backend,
	entrypoint: String,
	version: String,
	#[serde(default)]
	min_app_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Backend {
	Wasm,
	Lua,
}

#[derive(serde::Serialize)]
struct RepoEntry {
	id: String,
	backend: &'static str,
	version: String,
	plugin_api: String,
	min_app_version: Option<String>,
	sha256: String,
	url: String,
}

fn main() {
	let mut args = std::env::args().skip(1);
	let repo_dir = std::env::current_dir()
		.expect("cwd")
		.join(args.next().unwrap_or_else(|| ".".into()));
	let base_url = args.next().unwrap_or_else(|| {
		"https://github.com/Dewn5228/manga-vault-plugins/releases/download/latest".into()
	});

	let repo_dir = repo_dir.canonicalize().expect("repo dir exists");
	let sources = repo_dir.join("sources");
	let dist = repo_dir.join("dist");
	std::fs::create_dir_all(&dist).expect("create dist");

	let mut entries = Vec::new();
	for dir in plugin_dirs(&sources) {
		match pack(&dir, &dist, &base_url) {
			Ok(entry) => entries.push(entry),
			Err(error) => {
				eprintln!("skip {}: {error}", dir.display());
			},
		}
	}

	entries.sort_by(|a, b| a.id.cmp(&b.id));
	let manifest = serde_json::json!({
		"name": "manga-vault-community",
		"updated_at": chrono::Utc::now().to_rfc3339(),
		"plugins": entries,
	});
	let path = dist.join("repo.json");
	std::fs::write(
		&path,
		serde_json::to_string_pretty(&manifest).expect("serialize repo.json") + "\n",
	)
	.expect("write repo.json");
	println!("wrote {} ({} plugins)", path.display(), entries.len());
}

fn plugin_dirs(sources: &Path) -> Vec<PathBuf> {
	let mut dirs = Vec::new();
	for entry in std::fs::read_dir(sources).expect("sources dir") {
		let path = entry.expect("entry").path();
		if path.join("plugin.toml").is_file() {
			dirs.push(path);
		}
	}
	dirs.sort();
	dirs
}

fn pack(dir: &Path, dist: &Path, base_url: &str) -> Result<RepoEntry, String> {
	let raw = std::fs::read_to_string(dir.join("plugin.toml")).map_err(|e| e.to_string())?;
	let manifest: Manifest = toml::from_str(&raw).map_err(|e| e.to_string())?;

	match manifest.backend {
		Backend::Wasm => ensure_wasm_built(dir, &manifest.entrypoint)?,
		Backend::Lua => {
			if !dir.join(&manifest.entrypoint).is_file() {
				return Err(format!("lua entrypoint {} missing", manifest.entrypoint));
			}
		},
	}

	if let Some(api) = api_major_of(dir) {
		_ = api;
	}

	let artifact_name = format!("{}-{}.mvplug", manifest.id, manifest.version);
	let artifact_path = dist.join(&artifact_name);
	let bundle = bundle(dir, &manifest)?;
	std::fs::write(&artifact_path, &bundle).map_err(|e| e.to_string())?;

	let digest = sha2::Sha256::digest(&bundle);
	let sha256 = hex_digest(&digest);

	Ok(RepoEntry {
		id: manifest.id.clone(),
		backend: match manifest.backend {
			Backend::Wasm => "wasm",
			Backend::Lua => "lua",
		},
		version: manifest.version.clone(),
		plugin_api: plugin_api(dir)?,
		min_app_version: manifest.min_app_version.clone(),
		sha256,
		url: format!("{base_url}/{artifact_name}"),
	})
}

fn ensure_wasm_built(dir: &Path, entrypoint: &str) -> Result<(), String> {
	let artifact = dir.join(entrypoint);
	let needs_build = match newest_source_mtime(dir) {
		Some(mtime) => !artifact.is_file() || Some(mtime) > file_mtime(&artifact),
		None => !artifact.is_file(),
	};

	if needs_build {
		println!("building {}", dir.display());
		let status = Command::new("cargo")
			.args(["build", "--target", WASM_TARGET, "--release"])
			.current_dir(dir)
			.status()
			.map_err(|e| e.to_string())?;
		if !status.success() {
			return Err("cargo build failed".into());
		}
		let package = package_artifact_name(dir)?;
		let built = dir
			.join("target")
			.join(WASM_TARGET)
			.join("release")
			.join(format!("{package}.wasm"));
		std::fs::copy(&built, &artifact).map_err(|e| e.to_string())?;
	}
	Ok(())
}

fn newest_source_mtime(dir: &Path) -> Option<std::time::SystemTime> {
	walk(dir).filter(|path| is_source(path)).filter_map(|path| file_mtime(&path)).max()
}

fn walk(dir: &Path) -> impl Iterator<Item = PathBuf> {
	std::fs::read_dir(dir)
		.into_iter()
		.flatten()
		.filter_map(Result::ok)
		.flat_map(|entry| {
			let path = entry.path();
			if path.is_dir() && path.file_name().is_some_and(|name| name != "target") {
				walk(&path).chain(std::iter::once(path)).collect::<Vec<_>>()
			} else {
				vec![path]
			}
		})
}

fn is_source(path: &Path) -> bool {
	path.extension().is_some_and(|ext| ext == "rs" || ext == "toml")
}

fn file_mtime(path: &Path) -> Option<std::time::SystemTime> {
	std::fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

fn package_artifact_name(dir: &Path) -> Result<String, String> {
	let text = std::fs::read_to_string(dir.join("Cargo.toml")).map_err(|e| e.to_string())?;
	text.lines()
		.find_map(|line| line.trim_start().strip_prefix("name = \""))
		.and_then(|rest| rest.split('"').next())
		.map(|name| name.replace('-', "_"))
		.ok_or_else(|| "package name missing".into())
}

fn plugin_api(dir: &Path) -> Result<String, String> {
	let text = std::fs::read_to_string(dir.join("plugin.toml")).map_err(|e| e.to_string())?;
	text.lines()
		.find_map(|line| line.trim_start().strip_prefix("plugin_api = \""))
		.and_then(|rest| rest.split('"').next())
		.map(str::to_owned)
		.ok_or_else(|| "plugin_api missing".into())
}

fn api_major_of(dir: &Path) -> Option<u64> {
	plugin_api(dir).ok()?.split('.').next()?.parse().ok()
}

fn bundle(dir: &Path, manifest: &Manifest) -> Result<Vec<u8>, String> {
	let mut tar_bytes = Vec::new();
	{
		let mut builder = tar::Builder::new(&mut tar_bytes);
		append_file(&mut builder, dir, &format!("{}/plugin.toml", manifest.id), "plugin.toml")?;
		append_file(
			&mut builder,
			dir,
			&format!("{}/{}", manifest.id, manifest.entrypoint),
			&manifest.entrypoint,
		)?;
		builder.finish().map_err(|e| e.to_string())?;
	}
	let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
	gz.write_all(&tar_bytes).map_err(|e| e.to_string())?;
	gz.finish().map_err(|e| e.to_string())
}

fn append_file(
	builder: &mut tar::Builder<&mut Vec<u8>>,
	dir: &Path,
	name: &str,
	relative: &str,
) -> Result<(), String> {
	let data = std::fs::read(dir.join(relative)).map_err(|e| e.to_string())?;
	let mut header = tar::Header::new_gnu();
	header.set_size(data.len() as u64);
	header.set_mode(0o644);
	header.set_cksum();
	builder.append_data(&mut header, name, data.as_slice()).map_err(|e| e.to_string())
}

fn hex_digest(digest: &[u8]) -> String {
	digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
