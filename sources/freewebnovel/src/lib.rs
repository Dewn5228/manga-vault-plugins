wit_bindgen::generate!({
	path: "../../wit",
	world: "source-world",
});

use exports::manga_vault::source::source::{
	Guest, SourceError, SourceInfo, WorkDetails, WorkSummary,
};
use manga_vault::source::{flare_solverr, html, http};
use manga_vault::source::types::{Chapter, WorkKind};

const BASE: &str = "https://freewebnovel.com";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36";

fn http_get(url: &str) -> Result<String, SourceError> {
	let headers = vec![
		http::Header { name: "Content-Type".to_string(), value: "application/x-www-form-urlencoded".to_string() },
		http::Header { name: "Referer".to_string(), value: format!("{BASE}/home") },
		http::Header { name: "Origin".to_string(), value: BASE.to_string() },
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

struct Freewebnovel;

impl Guest for Freewebnovel {
	fn get_info() -> SourceInfo {
		SourceInfo {
			id: "freewebnovel".to_string(),
			name: "FreeWebNovel".to_string(),
			version: "1.0.0".to_string(),
			kind: WorkKind::Novel,
			icon_url: None,
			referer_url: Some(BASE.to_string()),
			base_url: Some(BASE.to_string()),
		}
	}

	fn search(query: String, page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 1 {
			return Ok(vec![]);
		}
		let body = http_get(&format!(
			"{BASE}/search?keyword={}",
			urlencoding::encode(&query)
		))?;
		Ok(parse_grid(&body))
	}

	fn latest(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 10 {
			return Ok(vec![]);
		}
		let body = http_get(&format!("{BASE}/sort/latest-release?p={page}"))?;
		Ok(parse_grid(&body))
	}

	fn trending(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 1 {
			return Ok(vec![]);
		}
		let body = http_get(&format!("{BASE}/sort/most-popular?p={page}"))?;
		Ok(parse_grid(&body))
	}

	fn fetch_work(url: String) -> Result<WorkDetails, SourceError> {
		let body = http_get(&url)?;

		let title = html::find_one(&body, "h1")
			.map(|element| html::text(&element))
			.unwrap_or_else(|| url.to_string());

		let mut authors = Vec::new();
		for element in html::find(&body, "[itemprop='author'] a") {
			authors.push(html::text(&element));
		}

		let description = html::find_one(&body, "#description-box")
			.map(|element| html::text(&element))
			.filter(|description| !description.is_empty());

		let mut genres = Vec::new();
		for element in html::find(&body, "a[href*='/genre/']") {
			genres.push(html::text(&element));
		}

		let status = html::find_one(&body, ".header-stats span:last-of-type")
			.map(|element| html::text(&element))
			.filter(|status| !status.is_empty());

		let mut chapters = Vec::new();
		chapters.extend(parse_chapters(&body, "#idData a"));
		let total_pages = html::find_one(&body, "#indexListPage")
			.and_then(|element| html::attr(&element, "data-total-page"))
			.and_then(|pages| pages.parse::<u32>().ok())
			.unwrap_or(1);
		for page in 2..=total_pages {
			std::thread::sleep(std::time::Duration::from_millis(750));
			let page_url = format!(
				"{}{}ajax=chapters&page={page}&pageSize=40",
				url,
				if url.contains('?') { '&' } else { '?' },
			);
			let page_body = http_get(&page_url)?;
			let page_json = serde_json::from_str::<serde_json::Value>(&page_body)
				.map_err(|error| SourceError::Parse(error.to_string()))?;
			let page_html = page_json
				.get("html")
				.and_then(serde_json::Value::as_str)
				.ok_or_else(|| SourceError::Parse("missing chapter page html".into()))?;
			chapters.extend(parse_chapters(page_html, "a[href*='/chapter-']"));
		}
		chapters.reverse();

		Ok(WorkDetails {
			title,
			url,
			cover_url: None,
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
		let paragraphs: Vec<String> = html::find(&body, "#article")
			.into_iter()
			.map(|element| html::text(&element))
			.filter(|text| !text.trim().is_empty())
			.collect();
		Ok(paragraphs)
	}
}

fn parse_chapters(body: &str, selector: &str) -> Vec<Chapter> {
	html::find(body, selector)
		.into_iter()
		.map(|element| {
			let chapter_url = html::attr(&element, "href").unwrap_or_default();
			Chapter {
				title: html::text(&element),
				url: format!("{BASE}{chapter_url}"),
				date: None,
				scanlation_group: None,
			}
		})
		.collect()
}

fn parse_grid(body: &str) -> Vec<WorkSummary> {
	let mut works: Vec<WorkSummary> = Vec::new();
	for element in html::find(body, "a[href*='/novel/']") {
		let href = match html::attr(&element, "href") {
			Some(href) if href.starts_with("/novel/") && !href.contains("/chapter-") => href,
			_ => continue,
		};
		let title = html::text(&element);
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

export!(Freewebnovel);
