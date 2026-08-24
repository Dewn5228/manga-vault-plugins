wit_bindgen::generate!({
	path: "../../wit",
	world: "source-world",
});

use exports::manga_vault::source::source::{
	Guest, SourceError, SourceInfo, WorkDetails, WorkSummary,
};
use manga_vault::source::types::{Chapter, WorkKind};
use manga_vault::source::{flare_solverr, html, http};

const BASE: &str = "https://www.mangaread.org";
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

fn listing(path_and_query: String) -> Result<Vec<WorkSummary>, SourceError> {
	let body = http_get(&path_and_query)?;
	Ok(parse_items(&body))
}

struct MangareadOrg;

impl Guest for MangareadOrg {
	fn get_info() -> SourceInfo {
		SourceInfo {
			id: "mangaread_org".to_string(),
			name: "MangaRead".to_string(),
			version: "1.0.0".to_string(),
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
		listing(format!(
			"{BASE}/?s={}&post_type=wp-manga",
			urlencoding::encode(&query)
		))
	}

	fn latest(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 100 {
			return Ok(vec![]);
		}
		listing(format!(
			"{BASE}/manga/?m_orderby=latest&paged={page}"
		))
	}

	fn trending(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 20 {
			return Ok(vec![]);
		}
		listing(format!(
			"{BASE}/manga/?m_orderby=views&paged={page}"
		))
	}

	fn fetch_work(url: String) -> Result<WorkDetails, SourceError> {
		let body = http_get(&url)?;

		let title = html::find_one(&body, "h1")
			.map(|element| html::text(&element))
			.unwrap_or_else(|| url.to_string());

		let mut authors = Vec::new();
		for element in html::find(&body, ".author-content a") {
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
			.and_then(|element| {
				html::attr(&element, "data-src").or_else(|| html::attr(&element, "src"))
			})
			.map(|src| if src.starts_with("http") { src } else { format!("https:{src}") });

		let mut chapters = Vec::new();
		for element in html::find(&body, "li.wp-manga-chapter a") {
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

fn is_work_href(href: &str) -> bool {
	let Some(rest) = href.split("/manga/").nth(1) else {
		return false;
	};
	let slug = rest.split('?').next().unwrap_or_default().trim_end_matches('/');
	!slug.is_empty()
		&& !slug.contains('/')
		&& slug != "feed"
		&& !rest.contains("/chapter-")
}

fn parse_items(body: &str) -> Vec<WorkSummary> {
	let mut works: Vec<WorkSummary> = Vec::new();
	for element in html::find(body, "a[href*='/manga/']") {
		let href = match html::attr(&element, "href") {
			Some(href) if is_work_href(&href) => href,
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
		let title = title
			.split(" on Hari")
			.next()
			.unwrap_or(&title)
			.replace("Read ", "")
			.replace(" Manga Online", "")
			.trim()
			.to_string();
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

export!(MangareadOrg);
