wit_bindgen::generate!({
	path: "../../wit",
	world: "source-world",
});

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::aes::cipher::consts::U16;
use aes_gcm::aes::Aes256;
use aes_gcm::{AesGcm, Key, Nonce};
use base64::Engine;
use exports::manga_vault::source::source::{Guest, SourceError, SourceInfo, WorkDetails, WorkSummary};
use manga_vault::source::types::{Chapter, WorkKind};
use manga_vault::source::{flare_solverr, html, http};

const BASE: &str = "https://wtr-lab.com";
const UA: &str =
	"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36";

fn headers(content_type: Option<&str>) -> Vec<http::Header> {
	let mut headers = vec![
		http::Header { name: "Referer".into(), value: format!("{BASE}/") },
		http::Header { name: "User-Agent".into(), value: UA.into() },
	];
	if let Some(value) = content_type {
		headers.push(http::Header { name: "Content-Type".into(), value: value.into() });
	}
	headers
}

fn get(url: &str) -> Result<String, SourceError> {
	let response = http::get(url, Some(&headers(None))).map_err(SourceError::Network)?;
	let Some(response) = response else { return Err(SourceError::Network("empty response".into())) };
	if http::has_cloudflare_protection(&response.body, Some(response.status), Some(&response.headers)) {
		if let Some(solved) = flare_solverr::get(url, None).map_err(SourceError::Network)? {
			return Ok(solved.body);
		}
	}
	match response.status {
		200 => Ok(response.body),
		404 => Err(SourceError::NotFound),
		status => Err(SourceError::Network(format!("HTTP {status} for {url}"))),
	}
}

fn post(url: &str, body: &str) -> Result<String, SourceError> {
	let response = http::post(url, body, Some(&headers(Some("application/json"))))
		.map_err(SourceError::Network)?
		.ok_or_else(|| SourceError::Network("empty response".into()))?;
	match response.status {
		200 => Ok(response.body),
		status => Err(SourceError::Network(format!("HTTP {status} for {url}"))),
	}
}

fn absolute(url: &str) -> String {
	if url.starts_with("http") { url.into() } else { format!("{BASE}{url}") }
}

fn parse_items(body: &str) -> Vec<WorkSummary> {
	let mut works = Vec::new();
	for card in html::find(body, ".series-list > div") {
		let Some(link) = html::find_one(&card.html, "a[href^='/en/novel/']") else { continue };
		let Some(url) = html::attr(&link, "href") else { continue };
		let title = html::attr(&link, "title")
			.or_else(|| html::find_one(&link.html, "h3").map(|e| html::text(&e)))
			.unwrap_or_default();
		if title.trim().is_empty() { continue; }
		let cover_url = html::find_one(&card.html, ".image-wrap img[alt]")
			.or_else(|| html::find_one(&card.html, "img"))
			.and_then(|image| html::attr(&image, "src"))
			.map(|src| absolute(&src));
		let summary = WorkSummary { title, url: absolute(&url), cover_url };
		if !works.iter().any(|work: &WorkSummary| work.url == summary.url) { works.push(summary); }
	}
	works
}

fn page_props(body: &str) -> Option<serde_json::Value> {
	let script = html::find_one(body, "script#__NEXT_DATA__")?;
	let value = serde_json::from_str::<serde_json::Value>(&html::text(&script)).ok()?;
	Some(value["props"]["pageProps"].clone())
}

fn chapters(body: &str, url: &str) -> Vec<Chapter> {
	let Some(props) = page_props(body) else { return Vec::new() };
	let data = &props["serie"]["serie_data"];
	let Some(raw_id) = data["raw_id"].as_i64() else { return Vec::new() };
	let Some(count) = data["raw_chapter_count"].as_i64() else { return Vec::new() };
	let Ok(body) = get(&format!("{BASE}/api/chapters/{raw_id}?start=1&end={count}")) else { return Vec::new() };
	let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) else { return Vec::new() };
	value["chapters"].as_array().into_iter().flatten().filter_map(|chapter| {
		Some(Chapter {
			title: format!("#{} {}", chapter["order"].as_i64()?, chapter["title"].as_str()?),
			url: format!("{}/chapter-{}", url.trim_end_matches('/'), chapter["order"].as_i64()?),
			date: chapter["updated_at"].as_str().map(str::to_string),
			scanlation_group: None,
		})
	}).collect()
}

fn decrypt(raw: &str) -> Result<Vec<String>, SourceError> {
	let array = raw.strip_prefix("arr:").is_some();
	let raw = raw.strip_prefix("arr:").or_else(|| raw.strip_prefix("str:")).unwrap_or(raw);
	let parts: Vec<_> = raw.split(':').collect();
	if parts.len() != 3 { return Err(SourceError::Parse("invalid encrypted chapter format".into())); }
	let engine = base64::engine::general_purpose::STANDARD;
	let iv = engine.decode(parts[0]).map_err(|e| SourceError::Parse(e.to_string()))?;
	let short = engine.decode(parts[1]).map_err(|e| SourceError::Parse(e.to_string()))?;
	let long = engine.decode(parts[2]).map_err(|e| SourceError::Parse(e.to_string()))?;
	let mut ciphertext = long;
	ciphertext.extend(short);
	let key = Key::<AesGcm<Aes256, U16>>::from_slice(b"IJAFUUxjM25hyzL2AZrn0wl7cESED6Ru");
	let plaintext = AesGcm::<Aes256, U16>::new(key).decrypt(Nonce::from_slice(&iv), ciphertext.as_ref())
		.map_err(|e| SourceError::Parse(e.to_string()))?;
	let text = String::from_utf8(plaintext).map_err(|e| SourceError::Parse(e.to_string()))?;
	if array { serde_json::from_str(&text).map_err(|e| SourceError::Parse(e.to_string())) } else { Ok(vec![text]) }
}

struct WtrLab;

impl Guest for WtrLab {
	fn get_info() -> SourceInfo {
		SourceInfo { id: "wtr_lab".into(), name: "WTR-LAB".into(), version: "1.0.0".into(), kind: WorkKind::Novel,
			icon_url: None, referer_url: Some(format!("{BASE}/")), base_url: Some(BASE.into()) }
	}

	fn search(query: String, _page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		Ok(parse_items(&get(&format!("{BASE}/en/novel-finder?text={}", urlencoding::encode(&query)))?))
	}

	fn latest(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		Ok(parse_items(&get(&format!("{BASE}/en/novel-list?page={page}&status=all&orderBy=date&genre="))?))
	}

	fn trending(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		Ok(parse_items(&get(&format!("{BASE}/en/novel-list?page={page}&status=all&orderBy=view&genre="))?))
	}

	fn fetch_work(url: String) -> Result<WorkDetails, SourceError> {
		let body = get(&url)?;
		let props = page_props(&body).ok_or_else(|| SourceError::Parse("missing WTR-LAB metadata".into()))?;
		let data = &props["serie"]["serie_data"];
		let title = data["data"]["raw"]["title"].as_str().or_else(|| data["data"]["title"].as_str()).unwrap_or(&url).to_string();
		let cover_url = data["data"]["image"].as_str().map(absolute);
		let description = data["data"]["description"].as_str().map(str::to_string);
		let authors = data["data"]["author"].as_str().map(|value| vec![value.to_string()]).unwrap_or_default();
		Ok(WorkDetails { title, url: url.clone(), cover_url, alternative_names: vec![], authors, artists: vec![], status: None,
			release_date: None, description, genres: vec![], chapters: chapters(&body, &url), content_html: None })
	}

	fn fetch_chapter(url: String) -> Result<Vec<String>, SourceError> {
		let body = get(&url)?;
		let props = page_props(&body).ok_or_else(|| SourceError::Parse("missing chapter metadata".into()))?;
		let serie = &props["serie"];
		let data = &serie["serie_data"];
		let chapter_id = serie["chapter"]["id"].as_i64().ok_or_else(|| SourceError::Parse("missing chapter id".into()))?;
		let chapter_no = serie["chapter"]["slug"].as_str().or_else(|| serie["chapter"]["name"].as_str()).unwrap_or_default();
		let raw_id = data["raw_id"].as_i64().ok_or_else(|| SourceError::Parse("missing series id".into()))?;
		let payload = serde_json::json!({"chapter_id": chapter_id, "chapter_no": chapter_no, "force_retry": "false", "language": "en", "raw_id": raw_id, "retry": "false", "translate": "web"});
		let response = serde_json::from_str::<serde_json::Value>(&post(&format!("{BASE}/api/reader/get"), &payload.to_string())?)
			.map_err(|e| SourceError::Parse(e.to_string()))?;
		let encrypted = response["data"]["data"]["body"].as_str().ok_or_else(|| SourceError::Parse("missing chapter body".into()))?;
		decrypt(encrypted)
	}
}

export!(WtrLab);
