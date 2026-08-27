wit_bindgen::generate!({
	path: "../../wit",
	world: "source-world",
});

use exports::manga_vault::source::source::{
	Guest, SourceError, SourceInfo, WorkDetails, WorkSummary,
};
use manga_vault::source::types::{Chapter, WorkKind};
use manga_vault::source::{flare_solverr, html, http};

const BASE: &str = "https://www.natomanga.com";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36";

fn http_get(url: &str) -> Result<String, SourceError> {
	let headers = vec![
		http::Header { name: "Referer".to_string(), value: format!("{BASE}/") },
		http::Header { name: "User-Agent".to_string(), value: UA.to_string() },
	];

	let response = http::get(url, Some(&headers)).map_err(|error| SourceError::Network(error))?;

	let Some(response) = response else {
		return Err(SourceError::Network("empty response".into()));
	};

	if http::has_cloudflare_protection(
		&response.body,
		Some(response.status),
		Some(&response.headers),
	) {
		if let Some(solved) = flare_solverr::get(url, None).map_err(|e| SourceError::Network(e))? {
			return Ok(solved.body);
		}
	}

	match response.status {
		200 => Ok(response.body),
		404 => Err(SourceError::NotFound),
		status => Err(SourceError::Network(format!("HTTP {status} for {url}"))),
	}
}

struct Natomanga;

impl Guest for Natomanga {
	fn get_info() -> SourceInfo {
		SourceInfo {
			id: "natomanga".to_string(),
			name: "NatoManga".to_string(),
			version: "1.0.1".to_string(),
			kind: WorkKind::Manga,
			icon_url: None,
			referer_url: Some(format!("{BASE}/")),
			base_url: Some(BASE.to_string()),
		}
	}

	fn search(query: String, page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 50 {
			return Ok(vec![]);
		}
		let dashed = query.replace(' ', "_");
		let encoded = urlencoding::encode(&dashed);
		let body = http_get(&format!("{BASE}/search/story/{encoded}?page={page}"))?;
		Ok(parse_items(&body))
	}

	fn latest(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 100 {
			return Ok(vec![]);
		}
		let body = http_get(&format!("{BASE}/manga-list/latest-manga?page={page}"))?;
		Ok(parse_items(&body))
	}

	fn trending(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 20 {
			return Ok(vec![]);
		}
		let body = http_get(&format!("{BASE}/manga-list/hot-manga?page={page}"))?;
		Ok(parse_items(&body))
	}

	fn fetch_work(url: String) -> Result<WorkDetails, SourceError> {
		let body = http_get(&url)?;

		let title = html::find_one(&body, "ul.manga-info-text li h1")
			.or_else(|| html::find_one(&body, "h1"))
			.map(|element| html::text(&element))
			.unwrap_or_else(|| url.to_string());

		let mut authors = Vec::new();
		let mut status = None;
		for element in html::find(&body, "ul.manga-info-text li") {
			let text = html::text(&element);
			if let Some(value) = text.strip_prefix("Author(s)") {
				let value = value.trim_start_matches(|c| c == ' ' || c == ':').trim();
				if !value.is_empty() && value != "Unknown" && !authors.contains(&value.to_string()) {
					authors.push(value.to_string());
				}
			} else if text.starts_with("Status") {
				status = text.split(':').nth(1).map(|value| value.trim().to_string());
			}
		}

		let alternative_names = html::find_one(&body, "h2.story-alternative")
			.map(|element| html::text(&element))
			.map(|text| {
				text.split_once(':')
					.map(|(_, rest)| rest.trim().to_string())
					.unwrap_or(text)
			})
			.map(|text| {
				text.split(',')
					.map(|name| name.trim().to_string())
					.filter(|name| !name.is_empty())
					.collect::<Vec<_>>()
			})
			.unwrap_or_default();

		let description = html::find_one(&body, ".description")
			.map(|element| html::text(&element))
			.filter(|description| !description.is_empty());

		let mut genres = Vec::new();
		for element in html::find(&body, "ul.manga-info-text li.genres a") {
			let genre = html::text(&element);
			if !genre.is_empty() && !genres.contains(&genre) {
				genres.push(genre);
			}
		}

		let cover_url = html::find_one(&body, ".manga-info-pic img")
			.and_then(|element| html::attr(&element, "src"));

		let slug = url.rsplit('/').next().unwrap_or_default().to_string();
		let mut chapters = chapters_from_api(&format!("{BASE}/api/manga/{slug}/chapters"), &url);
		if chapters.is_empty() {
			for element in html::find(&body, "#chapter-list-container .row a") {
				let chapter_url = html::attr(&element, "href").unwrap_or_default();
				chapters.push(Chapter {
					title: html::text(&element),
					url: chapter_url,
					date: None,
					scanlation_group: None,
				});
			}
		}
		chapters.reverse();

		Ok(WorkDetails {
			title,
			url,
			cover_url,
			alternative_names,
			authors,
			artists: vec![],
			status,
			release_date: None,
			description,
			genres,
			chapters,
			content_html: None,
		})
	}

	fn fetch_chapter(url: String) -> Result<Vec<String>, SourceError> {
		let body = http_get(&url)?;
		if let (Some(cdn), Some(paths)) = (js_strings(&body, "cdns"), js_strings(&body, "chapterImages")) {
			let cdn = cdn.first().map(|value| value.trim_end_matches('/'));
			let images: Vec<_> = cdn
				.into_iter()
				.flat_map(|cdn| paths.iter().map(move |path| format!("{cdn}/{}", path.trim_start_matches('/'))))
				.collect();
			if !images.is_empty() {
				return Ok(images);
			}
		}
		let mut images = Vec::new();
		for selector in ["div.vung-doc img", "div.container-chapter-reader img"] {
			for element in html::find(&body, selector) {
				let src = html::attr(&element, "src")
					.or_else(|| html::attr(&element, "data-src"))
					.unwrap_or_default();
				if src.starts_with("http") && !images.contains(&src) {
					images.push(src);
				}
			}
			if !images.is_empty() {
				break;
			}
		}
		if images.is_empty() {
			return Err(SourceError::Parse("no images found in chapter page".into()));
		}
		Ok(images)
	}
}

fn pace() {
	std::thread::sleep(std::time::Duration::from_millis(300));
}

fn parse_items(body: &str) -> Vec<WorkSummary> {
	let mut works: Vec<WorkSummary> = Vec::new();
	for element in html::find(body, "a[href*='/manga/']") {
		let href = match html::attr(&element, "href") {
			Some(href)
				if href.contains("/manga/")
					&& !href.contains("/chapter-")
					&& !href.contains("/genre") =>
			{
				href
			}
			_ => continue,
		};
		let title = match html::attr(&element, "title") {
			Some(title) if !title.is_empty() => title,
			_ => html::text(&element),
		};
		let url = if href.starts_with("http") {
			href
		} else {
			format!("{BASE}{href}")
		};
		if title.is_empty() || works.iter().any(|work| work.url == url) {
			continue;
		}
		works.push(WorkSummary {
			title,
			url,
			cover_url: None,
		});
	}
	works
}

fn chapters_from_api(api_url_base: &str, chapter_url_prefix: &str) -> Vec<Chapter> {
	let mut all = Vec::new();
	for page in 0..=200u32 {
		let body = match http_get(&format!("{api_url_base}?limit=50&offset={}", page * 50)) {
			Ok(body) => body,
			Err(_) => break,
		};
		pace();
		let Some(chapters) = extract_api_chapters(&body) else {
			break;
		};
		if chapters.is_empty() {
			break;
		}
		let has_more = body.contains("\"has_more\":true");
		for (name, slug) in chapters {
			all.push(Chapter {
				title: name,
				url: format!("{chapter_url_prefix}/{slug}"),
				date: None,
				scanlation_group: None,
			});
		}
		if !has_more {
			break;
		}
	}
	all
}

fn extract_api_chapters(body: &str) -> Option<Vec<(String, String)>> {
	let start = body.find("\"chapters\":")? + "\"chapters\":".len();
	let open = start + body[start..].find('[')?;
	let close = open + body[open..].rfind(']')?;
	let array = &body[open + 1..close];
	let mut out = Vec::new();
	for object in array.split("},{") {
		let Some(name) = extract_string_field(object, "chapter_name") else {
			continue;
		};
		let Some(slug) = extract_string_field(object, "chapter_slug") else {
			continue;
		};
		out.push((name, slug));
	}
	Some(out)
}

fn extract_string_field(text: &str, field: &str) -> Option<String> {
	let key = format!("\"{field}\":\"");
	let start = text.find(&key)? + key.len();
	let rest = &text[start..];
	let end = rest.find('"')?;
	Some(rest[..end].to_string())
}

fn js_strings(body: &str, field: &str) -> Option<Vec<String>> {
	let start = body.find(&format!("\"{field}\""))?;
	let open = start + body[start..].find('[')?;
	let close = open + body[open..].find(']')? + 1;
	serde_json::from_str(&body[open..close]).ok()
}

export!(Natomanga);
