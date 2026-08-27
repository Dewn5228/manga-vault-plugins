wit_bindgen::generate!({
	path: "../../wit",
	world: "source-world",
});

use exports::manga_vault::source::source::{Guest, SourceError, SourceInfo, WorkDetails, WorkSummary};
use manga_vault::source::types::{Chapter, WorkKind};
use manga_vault::source::{flare_solverr, html, http};

const BASE: &str = "https://www.readernovel.net";
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

fn browse(sort: &str, page: u32) -> Result<Vec<WorkSummary>, SourceError> {
	let body = http_get(&format!("{BASE}/browse?sort={sort}&status=0&p={page}"))?;
	Ok(parse_items(&body))
}

struct Readernovel;

impl Guest for Readernovel {
	fn get_info() -> SourceInfo {
		SourceInfo {
			id: "readernovel".to_string(),
			name: "ReaderNovel".to_string(),
			version: "1.0.1".to_string(),
			kind: WorkKind::Novel,
			icon_url: None,
			referer_url: Some(format!("{BASE}/")),
			base_url: Some(BASE.to_string()),
		}
	}

	fn search(query: String, page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 1 {
			return Ok(vec![]);
		}
		if let Some(hits) = search_via_api(&query)? {
			return Ok(hits);
		}
		browse("name", 1)
	}

	fn latest(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 500 {
			return Ok(vec![]);
		}
		browse("date", page)
	}

	fn trending(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 50 {
			return Ok(vec![]);
		}
		browse("popular", page)
	}

	fn fetch_work(url: String) -> Result<WorkDetails, SourceError> {
		let body = http_get(&url)?;

		let title = html::find_one(&body, "h1.page-title")
			.map(|element| html::text(&element))
			.unwrap_or_else(|| url.to_string());

		let mut authors = Vec::new();
		let mut status = None;
		for element in html::find(&body, "ul.list-group li.list-group-item") {
			let text = html::text(&element);
			let Some((label, value)) = text.split_once(':') else {
				continue;
			};
			let label = label.trim();
			let value = value.trim().to_string();
			if value.is_empty() {
				continue;
			}
			match label {
				"Author" | "Author(s)" | "Authors" => {
					if !value.eq_ignore_ascii_case("unknown") && !authors.contains(&value) {
						authors.push(value);
					}
				}
				"Status" => status = Some(value),
				_ => {}
			}
		}

		let description = html::find_one(&body, "#collapseSummary")
			.map(|element| html::text(&element))
			.filter(|description| !description.is_empty());

		let mut genres = Vec::new();
		for element in html::find(&body, "#collapseGenres a") {
			let genre = html::text(&element);
			if !genre.is_empty() && !genres.contains(&genre) {
				genres.push(genre);
			}
		}

		let cover_url = find_cover(&body);

		let mut chapters = Vec::new();
		for element in html::find(&body, ".chapter-list-wrapper a") {
			let Some(href) = html::attr(&element, "href") else {
				continue;
			};
			let title = match html::attr(&element, "title") {
				Some(title) if !title.is_empty() => title,
				_ => html::text(&element),
			};
			chapters.push(Chapter {
				title,
				url: if href.starts_with("http") {
					href
				} else {
					format!("{BASE}{href}")
				},
				date: None,
				scanlation_group: None,
			});
		}
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
		let paragraphs: Vec<String> = html::find(&body, "#chapter-container p")
			.into_iter()
			.map(|element| html::text(&element))
			.filter(|text| !text.trim().is_empty())
			.collect();
		if paragraphs.is_empty() {
			return Err(SourceError::Parse("no content found in chapter page".into()));
		}
		Ok(paragraphs)
	}
}

fn find_cover(body: &str) -> Option<String> {
	html::find_one(body, "meta[property='og:image']")
		.and_then(|element| html::attr(&element, "content"))
		.or_else(|| {
			html::find_one(body, ".book-cover img, .novel-cover img")
				.and_then(|element| html::attr(&element, "data-src").or_else(|| html::attr(&element, "src")))
		})
		.map(|url| match url.as_str() {
			url if url.starts_with("http://") || url.starts_with("https://") => url.to_owned(),
			url if url.starts_with("//") => format!("https:{url}"),
			url if url.starts_with('/') => format!("{BASE}{url}"),
			url => format!("{BASE}/{url}"),
		})
}

fn parse_items(body: &str) -> Vec<WorkSummary> {
	let mut works: Vec<WorkSummary> = Vec::new();
	for element in html::find(body, "a[href*='/novel/']") {
		let href = match html::attr(&element, "href") {
			Some(href) if href.starts_with("/novel/") => href,
			_ => continue,
		};
		let title = match html::attr(&element, "title") {
			Some(title) if !title.is_empty() => title,
			_ => html::text(&element),
		};
		let url = format!("{BASE}{href}");
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

fn search_via_api(query: &str) -> Result<Option<Vec<WorkSummary>>, SourceError> {
	let browse_body = http_get(&format!("{BASE}/browse?sort=date&status=0&p=1"))?;
	let Some(token) =
		html::find_one(&browse_body, "input.input-search").and_then(|element| html::attr(&element, "data-csrf"))
	else {
		return Ok(None);
	};

	let payload = format!(
		"{{\"query\":\"{}\",\"_token\":\"{}\"}}",
		query.replace('\\', "\\\\").replace('"', "\\\""),
		token
	);
	let headers = vec![
		http::Header {
			name: "Content-Type".to_string(),
			value: "application/json".to_string(),
		},
		http::Header {
			name: "Accept".to_string(),
			value: "application/json".to_string(),
		},
	];
	let response =
		http::post(&format!("{BASE}/search"), &payload, Some(&headers)).map_err(|error| SourceError::Network(error))?;

	let Some(response) = response else {
		return Ok(None);
	};
	if response.status != 200 {
		return Ok(None);
	}

	let mut works = Vec::new();
	for segment in response.body.split("{\"name\":").skip(1) {
		let Some(name) = extract_string_field(segment, "name") else {
			continue;
		};
		let Some(slug) = extract_string_field(segment, "slug") else {
			continue;
		};
		if name.is_empty() || slug.is_empty() {
			continue;
		}
		works.push(WorkSummary {
			title: name,
			url: format!("{BASE}/novel/{slug}"),
			cover_url: None,
		});
	}
	if works.is_empty() {
		return Ok(None);
	}
	Ok(Some(works))
}

fn extract_string_field(text: &str, field: &str) -> Option<String> {
	let key = format!("\"{field}\":\"");
	let start = text.find(&key)? + key.len();
	let rest = &text[start..];
	let end = rest.find('"')?;
	Some(rest[..end].to_string())
}

export!(Readernovel);
