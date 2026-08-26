wit_bindgen::generate!({
	path: "../../wit",
	world: "source-world",
});

use exports::manga_vault::source::source::{Guest, SourceError, SourceInfo, WorkDetails, WorkSummary};
use manga_vault::source::types::{Chapter, WorkKind};
use manga_vault::source::{flare_solverr, html, http};

const BASE: &str = "https://www.harimanga.co.uk";
const UA: &str =
	"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36";

fn http_get(url: &str) -> Result<String, SourceError> {
	let headers = vec![
		http::Header {
			name: "Referer".to_string(),
			value: format!("{BASE}/"),
		},
		http::Header {
			name: "User-Agent".to_string(),
			value: UA.to_string(),
		},
	];

	let response = http::get(url, Some(&headers)).map_err(|error| SourceError::Network(error))?;

	let Some(response) = response else {
		return Err(SourceError::Network("empty response".into()));
	};

	if http::has_cloudflare_protection(&response.body, Some(response.status), Some(&response.headers)) {
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

fn listing(path_and_query: String) -> Result<Vec<WorkSummary>, SourceError> {
	let body = http_get(&path_and_query)?;
	Ok(parse_items(&body))
}

struct HariManga;

impl Guest for HariManga {
	fn get_info() -> SourceInfo {
		SourceInfo {
			id: "hari_manga".to_string(),
			name: "HariManga".to_string(),
			version: "1.0.1".to_string(),
			kind: WorkKind::Manga,
			icon_url: None,
			referer_url: Some(format!("{BASE}/")),
			base_url: Some(BASE.to_string()),
		}
	}

	fn search(query: String, page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 1 {
			return Ok(vec![]);
		}
		listing(format!("{BASE}/?s={}&post_type=wp-manga", urlencoding::encode(&query)))
	}

	fn latest(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 100 {
			return Ok(vec![]);
		}
		listing(format!("{BASE}/home/page/{page}?orderby=latest&post_type=wp-manga"))
	}

	fn trending(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 20 {
			return Ok(vec![]);
		}
		listing(format!("{BASE}/home/page/{page}?orderby=trending&post_type=wp-manga"))
	}

	fn fetch_work(url: String) -> Result<WorkDetails, SourceError> {
		let body = http_get(&url)?;

		let title = html::find_one(&body, "h1")
			.map(|element| html::text(&element))
			.unwrap_or_else(|| url.to_string());

		let mut authors = Vec::new();
		for element in html::find(&body, "a[href*='/author']") {
			let author = html::text(&element);
			if !author.is_empty() && !authors.contains(&author) {
				authors.push(author);
			}
		}

		let mut summary_texts = Vec::new();
		for element in html::find(&body, ".summary-content") {
			summary_texts.push(html::text(&element));
		}

		let mut status = None;
		for candidate in ["Ongoing", "Completed", "Hiatus", "Dropped"] {
			if summary_texts.iter().any(|text| text.contains(candidate)) {
				status = Some(candidate.to_string());
				break;
			}
		}

		let description = summary_texts
			.iter()
			.max_by_key(|text| text.len())
			.filter(|text| !text.is_empty())
			.cloned();

		let mut genres = Vec::new();
		for element in html::find(&body, ".genres_content a[href*='/genre/'], a[href*='/genre/']") {
			let genre = html::text(&element);
			if !genre.is_empty() && !genres.contains(&genre) {
				genres.push(genre);
			}
		}

		let cover_url = html::find_one(&body, ".summary_image img")
			.and_then(|element| html::attr(&element, "data-src").or_else(|| html::attr(&element, "src")))
			.and_then(|src| normalize_image_url(&src));

		let slug = url.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_string();
		let mut chapters = chapters_from_api(&format!("{BASE}/api/comics/{slug}/chapters",), url.trim_end_matches('/'));
		chapters.reverse();

		Ok(WorkDetails {
			title,
			url,
			cover_url,
			alternative_names: vec![],
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
		let mut images = Vec::new();
		for element in html::find(&body, "img.wp-manga-chapter-img") {
			let src = html::attr(&element, "src")
				.or_else(|| html::attr(&element, "data-src"))
				.unwrap_or_default()
				.trim()
				.to_string();
			if let Some(src) = normalize_image_url(&src) {
				if !images.contains(&src) {
					images.push(src);
				}
			}
		}
		if images.is_empty() {
			return Err(SourceError::Parse("no images found in chapter page".into()));
		}
		Ok(images)
	}
}

fn parse_items(body: &str) -> Vec<WorkSummary> {
	let mut works: Vec<WorkSummary> = Vec::new();
	for element in html::find(body, "a[href*='/manga/']") {
		let href = match html::attr(&element, "href") {
			Some(href) if href.contains("/manga/") && !href.contains("/chapter-") => href,
			_ => continue,
		};
		let title = match html::attr(&element, "title") {
			Some(title) if !title.is_empty() && title != href => title,
			_ => match html::attr(&element, "alt") {
				Some(alt) if !alt.is_empty() => alt,
				_ => html::text(&element),
			},
		};
		let url = if href.starts_with("http") {
			href
		} else {
			format!("{BASE}{href}")
		};
		let title = clean_title(&title);
		let cover_url = html::find_one(&element.html, "img")
			.and_then(|image| html::attr(&image, "data-src").or_else(|| html::attr(&image, "src")))
			.and_then(|src| normalize_image_url(&src));
		if title.is_empty() {
			continue;
		}
		if let Some(work) = works.iter_mut().find(|work| work.url == url) {
			if is_placeholder_title(&work.title) && !is_placeholder_title(&title) {
				work.title = title;
			}
			if work.cover_url.is_none() {
				work.cover_url = cover_url;
			}
		} else {
			works.push(WorkSummary { title, url, cover_url });
		}
	}
	works.retain(|work| !is_placeholder_title(&work.title));
	works
}

fn is_placeholder_title(title: &str) -> bool {
	matches!(title.trim().to_ascii_lowercase().as_str(), "read" | "read manga" | "manga")
}

fn clean_title(raw: &str) -> String {
	let title = raw.split(" on Hari").next().unwrap_or(raw).trim();
	let title = title.strip_prefix("Read ").unwrap_or(title);
	title.strip_suffix(" Manga Online").unwrap_or(title).trim().to_string()
}

fn normalize_image_url(raw: &str) -> Option<String> {
	let raw = raw.trim();
	if raw.is_empty() {
		return None;
	}
	let url = if raw.starts_with("//") {
		format!("https:{raw}")
	} else if raw.starts_with('/') {
		format!("{BASE}{raw}")
	} else {
		raw.to_string()
	};
	let authority_start = url.find("://")? + 3;
	let path_start = url[authority_start..].find('/').map(|offset| authority_start + offset)?;
	let (authority, path) = url.split_at(path_start);
	Some(format!("{authority}/{}", path.trim_start_matches('/')))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn normalizes_image_urls() {
		assert_eq!(
			normalize_image_url("https://cdn.example.test//thumb/a.webp"),
			Some("https://cdn.example.test/thumb/a.webp".into())
		);
		assert_eq!(
			normalize_image_url("//cdn.example.test/thumb/a.webp"),
			Some("https://cdn.example.test/thumb/a.webp".into())
		);
	}

	#[test]
	fn recognizes_read_overlay_text() {
		assert!(is_placeholder_title("Read"));
		assert!(!is_placeholder_title("My Dad Is Too Strong"));
	}

	#[test]
	fn cleans_listing_titles() {
		assert_eq!(clean_title("Read My Dad Is Too Strong Manga Online"), "My Dad Is Too Strong");
	}
}

fn chapters_from_api(api_url: &str, work_url: &str) -> Vec<Chapter> {
	let body = match http_get(api_url) {
		Ok(body) => body,
		Err(_) => return Vec::new(),
	};
	let Some(start_index) = body.find("\"chapters\":") else {
		return Vec::new();
	};
	let array_start = start_index + body[start_index..].find('[').unwrap_or(0);
	let Some(close_rel) = body[array_start..].rfind(']') else {
		return Vec::new();
	};
	let array = &body[array_start + 1..array_start + close_rel];

	let mut chapters = Vec::new();
	for object in array.split("},{") {
		let Some(name) = extract_string_field(object, "chapter_name") else {
			continue;
		};
		let Some(slug) = extract_string_field(object, "chapter_slug") else {
			continue;
		};
		chapters.push(Chapter {
			title: name,
			url: format!("{work_url}/{slug}"),
			date: None,
			scanlation_group: None,
		});
	}
	chapters
}

fn extract_string_field(text: &str, field: &str) -> Option<String> {
	let key = format!("\"{field}\":\"");
	let start = text.find(&key)? + key.len();
	let rest = &text[start..];
	let end = rest.find('"')?;
	Some(rest[..end].to_string())
}

export!(HariManga);
