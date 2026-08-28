wit_bindgen::generate!({
	path: "../../wit",
	world: "source-world",
});

use exports::manga_vault::source::source::{Guest, SourceError, SourceInfo, WorkDetails, WorkSummary};
use manga_vault::source::types::{Chapter, WorkKind};
use manga_vault::source::{flare_solverr, html, http};

const BASE: &str = "https://www.wuxiabox.com";
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

struct WuxiaBox;
impl Guest for WuxiaBox {
	fn get_info() -> SourceInfo {
		SourceInfo {
			id: "wuxia_box".into(),
			name: "WuxiaBox".into(),
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
		let headers = vec![
			http::Header {
				name: "User-Agent".into(),
				value: UA.into(),
			},
			http::Header {
				name: "Referer".into(),
				value: format!("{BASE}/search.html"),
			},
			http::Header {
				name: "Content-Type".into(),
				value: "application/x-www-form-urlencoded".into(),
			},
		];
		let payload = format!("show=title&tempid=1&tbname=news&keyboard={}", urlencoding::encode(&query));
		let response = http::post(&format!("{BASE}/e/search/index.php"), &payload, Some(&headers))
			.map_err(SourceError::Network)?
			.ok_or_else(|| SourceError::Network("empty response".into()))?;
		Ok(parse_grid(&response.body))
	}

	fn latest(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		browse("all", "newstime", page)
	}

	fn trending(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		browse("all", "onclick", page)
	}

	fn fetch_work(url: String) -> Result<WorkDetails, SourceError> {
		let body = get(&url)?;
		let title = html::find_one(&body, "h1.novel-title")
			.map(|e| html::text(&e))
			.unwrap_or_else(|| url.clone());
		let id = url
			.trim_end_matches('/')
			.rsplit('/')
			.next()
			.unwrap_or_default()
			.trim_end_matches(".html");
		let chapters = parse_chapters(&get(&format!(
			"{BASE}/e/extend/fy.php?page=0&wjm={id}&X-Requested-With=XMLHttpRequest"
		))?);
		let authors = html::find_one(&body, "[itemprop='author']")
			.map(|e| vec![html::text(&e)])
			.unwrap_or_default();
		let description = html::find_one(&body, "p.description").map(|e| html::text(&e));
		Ok(WorkDetails {
			title,
			url,
			cover_url: html::find_one(&body, "meta[property='og:image']").and_then(|e| html::attr(&e, "content")),
			alternative_names: vec![],
			authors,
			artists: vec![],
			status: None,
			release_date: None,
			description,
			genres: vec![],
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

fn browse(genre: &str, order: &str, page: u32) -> Result<Vec<WorkSummary>, SourceError> {
	if page > 10 {
		return Ok(vec![]);
	}
	Ok(parse_grid(&get(&format!("{BASE}/list/{genre}/all-{order}-{page}.html"))?))
}

fn parse_grid(body: &str) -> Vec<WorkSummary> {
	html::find(body, "li.novel-item")
		.into_iter()
		.filter_map(|card| {
			let a = html::find_one(&card.html, "a[title]")?;
			let title = html::attr(&a, "title")
				.filter(|s| !s.is_empty())
				.or_else(|| html::find_one(&card.html, "h4.novel-title").map(|e| html::text(&e)))?;
			let href = html::attr(&a, "href")?;
			Some(WorkSummary {
				title,
				url: absolute(&href),
				cover_url: html::find_one(&card.html, "img")
					.and_then(|e| html::attr(&e, "data-src").or_else(|| html::attr(&e, "src")))
					.map(|url| absolute(&url)),
			})
		})
		.collect()
}

fn parse_chapters(body: &str) -> Vec<Chapter> {
	html::find(body, "ul.chapter-list li")
		.into_iter()
		.filter_map(|li| {
			let a = html::find_one(&li.html, "a")?;
			let url = html::attr(&a, "href")?;
			let title = html::find_one(&li.html, "strong.chapter-title")
				.map(|e| html::text(&e))
				.unwrap_or_default();
			Some(Chapter {
				title,
				url: absolute(&url),
				date: html::find_one(&li.html, "time").map(|e| html::text(&e)),
				scanlation_group: None,
			})
		})
		.collect()
}

fn absolute(url: &str) -> String {
	if url.starts_with("http") {
		url.into()
	} else {
		format!("{BASE}{url}")
	}
}

export!(WuxiaBox);
