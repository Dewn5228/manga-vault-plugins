wit_bindgen::generate!({
	path: "../../wit",
	world: "source-world",
});

use exports::manga_vault::source::source::{Guest, SourceError, SourceInfo, WorkDetails, WorkSummary};
use manga_vault::source::types::{Chapter, WorkKind};
use manga_vault::source::{flare_solverr, html, http};

const BASE: &str = "https://novelphoenix.com";
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

fn pace() {
	std::thread::sleep(std::time::Duration::from_millis(600));
}

struct Novelphoenix;

impl Guest for Novelphoenix {
	fn get_info() -> SourceInfo {
		SourceInfo {
			id: "novelphoenix".to_string(),
			name: "NovelPhoenix".to_string(),
			version: "1.0.1".to_string(),
			kind: WorkKind::Novel,
			icon_url: None,
			referer_url: Some(format!("{BASE}/")),
			base_url: Some(BASE.to_string()),
		}
	}

	fn search(query: String, page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 100 {
			return Ok(vec![]);
		}
		let body = http_get(&format!("{BASE}/search?keyword={}&page={page}", urlencoding::encode(&query)))?;
		Ok(parse_items(&body))
	}

	fn latest(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 500 {
			return Ok(vec![]);
		}
		let body = http_get(&format!("{BASE}/latest-release-novels?page={page}"))?;
		Ok(parse_items(&body))
	}

	fn trending(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 50 {
			return Ok(vec![]);
		}
		let body = http_get(&format!("{BASE}/genre-all/sort-popular/status-all/all-novel?page={page}"))?;
		Ok(parse_items(&body))
	}

	fn fetch_work(url: String) -> Result<WorkDetails, SourceError> {
		let body = http_get(&url)?;

		let title = html::find_one(&body, "h1.novel-title")
			.or_else(|| html::find_one(&body, "h1"))
			.map(|element| html::text(&element))
			.unwrap_or_else(|| url.to_string());

		let authors: Vec<String> = html::find(&body, "[itemprop='author']")
			.into_iter()
			.map(|element| html::text(&element))
			.filter(|author| !author.is_empty())
			.collect();

		let status = html::find_one(&body, "strong.ongoing")
			.map(|_| "Ongoing".to_string())
			.or_else(|| html::find_one(&body, "strong.completed").map(|_| "Completed".to_string()));

		let description = html::find_one(&body, ".summary .content")
			.map(|element| html::text(&element))
			.filter(|description| !description.is_empty());

		let mut genres = Vec::new();
		for element in html::find(&body, ".categories ul li a") {
			let genre = html::text(&element);
			if !genre.is_empty() && !genres.contains(&genre) {
				genres.push(genre);
			}
		}

		let cover_url = find_cover(&body);

		let mut chapters = Vec::new();
		let chapters_base = format!("{}/chapters", url.trim_end_matches('/'));
		for page in 1..=200u32 {
			let page_body = match http_get(&format!("{chapters_base}?page={page}")) {
				Ok(body) => body,
				Err(_) => break,
			};
			pace();
			let links = chapter_links(&page_body);
			if links.is_empty() {
				break;
			}
			chapters.extend(links);
		}
		if chapters.is_empty() {
			chapters = chapter_links(&body);
		}

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
		for selector in ["#content", ".chapter-content"] {
			let paragraphs: Vec<String> = html::find(&body, selector)
				.into_iter()
				.map(|element| html::text(&element))
				.filter(|text| !text.trim().is_empty())
				.collect();
			if !paragraphs.is_empty() {
				return Ok(paragraphs);
			}
		}
		Err(SourceError::Parse("no content found in chapter page".into()))
	}
}

fn find_cover(body: &str) -> Option<String> {
	html::find_one(body, "meta[property='og:image']")
		.and_then(|element| html::attr(&element, "content"))
		.or_else(|| {
			html::find_one(body, "figure.novel-cover img, .novel-cover img, .book-cover img")
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
	let mut elements = html::find(body, "a[href^='/novel/']");
	elements.extend(html::find(body, "a[href^='/novel/']"));
	for element in elements {
		let href = match html::attr(&element, "href") {
			Some(href) if !href.contains("/chapter-") => href,
			_ => continue,
		};
		let title = match html::attr(&element, "title") {
			Some(title) if !title.is_empty() => title,
			_ => html::text(&element),
		};
		let title = title.strip_suffix(" | NovelFire").unwrap_or(&title).trim().to_string();
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

fn chapter_links(body: &str) -> Vec<Chapter> {
	let mut chapters = Vec::new();
	for element in html::find(body, "ul.chapter-list li a") {
		let Some(href) = html::attr(&element, "href") else {
			continue;
		};
		let title = html::text(&element);
		if title.is_empty() {
			continue;
		}
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
	chapters
}

export!(Novelphoenix);
