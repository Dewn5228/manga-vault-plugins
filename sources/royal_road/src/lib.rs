wit_bindgen::generate!({
	path: "../../wit",
	world: "source-world",
});

use exports::manga_vault::source::source::{Guest, SourceError, SourceInfo, WorkDetails, WorkSummary};
use manga_vault::source::types::{Chapter, WorkKind};
use manga_vault::source::{flare_solverr, html, http};

const BASE: &str = "https://www.royalroad.com";
const UA: &str =
	"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36";

fn get(url: &str) -> Result<String, SourceError> {
	let headers = vec![
		http::Header {
			name: "User-Agent".into(),
			value: UA.into(),
		},
		http::Header {
			name: "Referer".into(),
			value: format!("{BASE}/"),
		},
	];
	let response = http::get(url, Some(&headers))
		.map_err(SourceError::Network)?
		.ok_or_else(|| SourceError::Network("empty response".into()))?;
	if http::has_cloudflare_protection(&response.body, Some(response.status), Some(&response.headers)) {
		if let Some(solved) = flare_solverr::get(url, None).map_err(SourceError::Network)? {
			return Ok(solved.body);
		}
	}
	match response.status {
		200 => Ok(response.body),
		404 => Err(SourceError::NotFound),
		s => Err(SourceError::Network(format!("HTTP {s} for {url}"))),
	}
}

struct RoyalRoad;
impl Guest for RoyalRoad {
	fn get_info() -> SourceInfo {
		SourceInfo {
			id: "royal_road".into(),
			name: "Royal Road".into(),
			version: "1.0.0".into(),
			kind: WorkKind::Novel,
			icon_url: None,
			referer_url: Some(BASE.into()),
			base_url: Some(BASE.into()),
		}
	}

	fn search(query: String, page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 1 {
			return Ok(vec![]);
		}
		Ok(parse_grid(&get(&format!(
			"{BASE}/fictions/search?title={}",
			urlencoding::encode(&query)
		))?))
	}

	fn latest(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		browse("latest-updates", page)
	}

	fn trending(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		browse("trending", page)
	}

	fn fetch_work(url: String) -> Result<WorkDetails, SourceError> {
		let body = get(&url)?;
		let title = html::find_one(&body, "h1.font-white")
			.map(|e| html::text(&e))
			.unwrap_or_else(|| url.clone());
		let chapters = html::find(&body, "a[href*='/chapter/']")
			.into_iter()
			.filter_map(|link| {
				let url = html::attr(&link, "href")?;
				let title = html::text(&link);
				if title.is_empty() {
					return None;
				}
				Some(Chapter {
					title,
					url: absolute(&url),
					date: None,
					scanlation_group: None,
				})
			})
			.collect();
		let authors = html::find_one(&body, "h4.font-white span a")
			.map(|e| vec![html::text(&e)])
			.unwrap_or_default();
		let genres = html::find(&body, "span.tags a")
			.into_iter()
			.map(|e| html::text(&e))
			.filter(|s| !s.is_empty())
			.collect();
		Ok(WorkDetails {
			title,
			url,
			cover_url: find_cover(&body),
			alternative_names: vec![],
			authors,
			artists: vec![],
			status: None,
			release_date: None,
			description: html::find_one(&body, "div.description").map(|e| html::text(&e)),
			genres,
			chapters,
			content_html: None,
		})
	}

	fn fetch_chapter(url: String) -> Result<Vec<String>, SourceError> {
		let body = get(&url)?;
		let content =
			html::find_one(&body, "div.chapter-content").ok_or_else(|| SourceError::Parse("no chapter content".into()))?;
		Ok(vec![content.html])
	}
}

fn browse(order: &str, page: u32) -> Result<Vec<WorkSummary>, SourceError> {
	if page > 1 && order == "trending" {
		return Ok(vec![]);
	}
	Ok(parse_grid(&get(&format!("{BASE}/fictions/{order}?page={page}"))?))
}

fn parse_grid(body: &str) -> Vec<WorkSummary> {
	html::find(body, "div.fiction-list-item")
		.into_iter()
		.filter_map(|card| {
			let a = html::find_one(&card.html, "h2.fiction-title a")?;
			let title = html::text(&a);
			let href = html::attr(&a, "href")?;
			if title.is_empty() {
				return None;
			}
			Some(WorkSummary {
				title,
				url: absolute(&href),
				cover_url: html::find_one(&card.html, "figure img").and_then(|e| html::attr(&e, "src")),
			})
		})
		.collect()
}

fn find_cover(body: &str) -> Option<String> {
	html::find_one(body, "meta[property='og:image']").and_then(|e| html::attr(&e, "content"))
}

fn absolute(url: &str) -> String {
	if url.starts_with("http") {
		url.into()
	} else {
		format!("{BASE}{url}")
	}
}

export!(RoyalRoad);
