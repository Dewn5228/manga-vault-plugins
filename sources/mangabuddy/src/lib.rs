wit_bindgen::generate!({
	path: "../../wit",
	world: "source-world",
});

use exports::manga_vault::source::source::{
	Guest, SourceError, SourceInfo, WorkDetails, WorkSummary,
};
use manga_vault::source::types::{Chapter, WorkKind};
use manga_vault::source::{flare_solverr, html, http};

const BASE: &str = "https://comizy.io";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/149.0.0.0 Safari/537.36";

const RESERVED_PATHS: [&str; 9] = [
	"lists", "ranking", "genres", "authors", "search", "latest", "static", "library", "settings",
];

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

struct Mangabuddy;

impl Guest for Mangabuddy {
	fn get_info() -> SourceInfo {
		SourceInfo {
			id: "mangabuddy".to_string(),
			name: "Mangabuddy".to_string(),
			version: "1.0.1".to_string(),
			kind: WorkKind::Manga,
			icon_url: None,
			referer_url: Some(format!("{BASE}/")),
			base_url: Some(BASE.to_string()),
		}
	}

	fn search(query: String, page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 20 {
			return Ok(vec![]);
		}
		let body = http_get(&format!(
			"{BASE}/search?q={}&page={page}",
			urlencoding::encode(&query)
		))?;
		Ok(parse_items(&body))
	}

	fn latest(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 10 {
			return Ok(vec![]);
		}
		let body = http_get(&format!("{BASE}/latest?page={page}"))?;
		Ok(parse_items(&body))
	}

	fn trending(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 1 {
			return Ok(vec![]);
		}
		let body = http_get(&format!("{BASE}/ranking"))?;
		Ok(parse_items(&body))
	}

	fn fetch_work(url: String) -> Result<WorkDetails, SourceError> {
		let body = http_get(&url)?;

		let title = html::find_one(&body, "h1")
			.map(|element| html::text(&element))
			.unwrap_or_else(|| url.to_string());

		let mut authors = Vec::new();
		for element in html::find(&body, "a[href*='/author']") {
			let author = html::text(&element);
			if !author.is_empty()
				&& !author.eq_ignore_ascii_case("updating")
				&& !authors.contains(&author)
			{
				authors.push(author);
			}
		}

		let description = html::find_one(&body, "meta[name='description']")
			.and_then(|element| html::attr(&element, "content"))
			.filter(|description| !description.is_empty());

		let mut genres = Vec::new();
		for element in html::find(&body, "a[href^='/genres/']") {
			let genre = html::text(&element);
			if !genre.is_empty() && !genres.contains(&genre) {
				genres.push(genre);
			}
		}

		let status = ["Ongoing", "Completed", "Hiatus", "Dropped"]
			.into_iter()
			.find(|candidate| body.contains(&format!("\"status\":\"{candidate}\"")))
			.map(|candidate| candidate.to_string());

		let cover_url = html::find_one(&body, "img[src*='/covers/']")
			.and_then(|element| html::attr(&element, "src"))
			.filter(|src| src.starts_with("http"));

		let path = work_path(&url);
		let mut chapters = next_data_chapters(&body);
		if chapters.is_empty() {
			for element in html::find(&body, "a[href*='/chapter-']") {
			let Some(href) = html::attr(&element, "href") else {
				continue;
			};
			if !href.contains(&path) {
				continue;
			}
			let title = html::text(&element);
			if !title.trim_start().starts_with("Chapter") {
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
		let json_images = next_data_images(&body);
		if !json_images.is_empty() {
			return Ok(json_images);
		}
		let mut images = Vec::new();
		for element in html::find(&body, "img[src*='.cmzcdn.']") {
			let Some(src) = html::attr(&element, "src") else {
				continue;
			};
			if src.starts_with("http") && !images.contains(&src) {
				images.push(src);
			}
		}
		if images.is_empty() {
			return Err(SourceError::Parse("no images found in chapter page".into()));
		}
		Ok(images)
	}
}

fn next_data_chapters(body: &str) -> Vec<Chapter> {
	let Some(script) = html::find_one(body, "script#__NEXT_DATA__") else {
		return Vec::new();
	};
	let script_text = html::text(&script);
	let Ok(value) = serde_json::from_str::<serde_json::Value>(&script_text) else {
		return Vec::new();
	};
	value["props"]["pageProps"]["initialManga"]["chapters"]
		.as_array()
		.into_iter()
		.flatten()
		.filter_map(|chapter| {
			Some(Chapter {
				title: chapter["name"].as_str()?.to_string(),
				url: absolute_url(chapter["url"].as_str()?),
				date: chapter["updatedAt"].as_str().map(str::to_string),
				scanlation_group: None,
			})
		})
		.collect()
}

fn absolute_url(path: &str) -> String {
	if path.starts_with("http") {
		path.to_string()
	} else {
		format!("{BASE}{path}")
	}
}

fn next_data_images(body: &str) -> Vec<String> {
	let Some(script) = html::find_one(body, "script#__NEXT_DATA__") else {
		return Vec::new();
	};
	let text = html::text(&script);
	let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
		return Vec::new();
	};
	value["props"]["pageProps"]["initialChapter"]["images"]
		.as_array()
		.into_iter()
		.flatten()
		.filter_map(|image| image.as_str().map(str::to_string))
		.collect()
}

fn work_path(url: &str) -> String {
	let without_base = url.strip_prefix(BASE).unwrap_or(url);
	let trimmed = without_base.trim_start_matches('/');
	match trimmed.find('/') {
		Some(end) => &trimmed[..end],
		None => trimmed,
	}
	.to_string()
}

fn parse_items(body: &str) -> Vec<WorkSummary> {
	let mut works: Vec<WorkSummary> = Vec::new();
	for element in html::find(body, "a[href^='/']") {
		let href = match html::attr(&element, "href") {
			Some(href) => href,
			None => continue,
		};
		let path = href.trim_end_matches('/');
		if path == "/" || path[1..].contains('/') || path.contains('.') {
			continue;
		}
		let first_segment = &path[1..];
		if RESERVED_PATHS.contains(&first_segment) {
			continue;
		}
		let title = match html::attr(&element, "title") {
			Some(title) if !title.is_empty() => title,
			_ => html::text(&element),
		};
		let url = format!("{BASE}{path}");
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

export!(Mangabuddy);
