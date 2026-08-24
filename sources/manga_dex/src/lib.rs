wit_bindgen::generate!({
	path: "../../wit",
	world: "source-world",
});

use exports::manga_vault::source::source::{
	Guest, SourceError, SourceInfo, WorkDetails, WorkSummary,
};
use manga_vault::source::types::{Chapter, WorkKind};
use manga_vault::source::{flare_solverr, http};

const API_BASE: &str = "https://api.mangadex.org";
const CDN_BASE: &str = "https://uploads.mangadex.org";
const SITE_BASE: &str = "https://mangadex.org";
const LIST_LIMIT: u32 = 20;
const FEED_LIMIT: u32 = 500;
const CONTENT_RATINGS: &[&str] = &["safe", "suggestive"];

fn http_get_json(url: &str) -> Result<serde_json::Value, SourceError> {
	let headers = vec![
		http::Header { name: "Referer".to_string(), value: format!("{SITE_BASE}/") },
		http::Header { name: "Origin".to_string(), value: SITE_BASE.to_string() },
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
			return serde_json::from_str(&solved.body)
				.map_err(|error| SourceError::Parse(error.to_string()));
		}
	}

	match response.status {
		200 => serde_json::from_str(&response.body).map_err(|error| SourceError::Parse(error.to_string())),
		404 => Err(SourceError::NotFound),
		status => Err(SourceError::Network(format!("HTTP {status} for {url}"))),
	}
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct MangaAttributes {
	title: std::collections::HashMap<String, String>,
	#[serde(default)]
	alt_titles: Vec<std::collections::HashMap<String, String>>,
	#[serde(default)]
	description: std::collections::HashMap<String, String>,
	#[serde(default)]
	tags: Vec<TagEntity>,
	#[serde(default)]
	status: Option<String>,
	#[serde(default)]
	year: Option<u16>,
}

#[derive(serde::Deserialize)]
struct TagEntity {
	id: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChapterAttributes {
	#[serde(default)]
	chapter: Option<String>,
	#[serde(default)]
	title: Option<String>,
	#[serde(default)]
	publish_at: String,
	#[serde(default)]
	external_url: Option<String>,
	#[serde(default)]
	pages: u32,
}

struct Relationship {
	attributes: serde_json::Value,
}

impl<'de> serde::Deserialize<'de> for Relationship {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		#[derive(serde::Deserialize)]
		struct Raw {
			#[allow(dead_code)]
			id: String,
			#[serde(rename = "type")]
			kind: String,
			#[serde(default)]
			attributes: Option<serde_json::Value>,
		}
		let raw = Raw::deserialize(deserializer)?;
		let _ = raw.kind;
		Ok(Relationship {
			attributes: raw.attributes.unwrap_or_default(),
		})
	}
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AtHome {
	base_url: String,
	chapter: AtHomeChapter,
}

#[derive(serde::Deserialize)]
struct AtHomeChapter {
	hash: String,
	data: Vec<String>,
}

fn relationship<'a>(
	body: &'a serde_json::Value,
	kind: &str,
) -> Option<Relationship> {
	body["relationships"]
		.as_array()?
		.iter()
		.filter(|entry| entry["type"] == kind)
		.find_map(|entry| serde_json::from_value(entry.clone()).ok())
}

fn localized(map: &std::collections::HashMap<String, String>) -> Option<String> {
	map.get("en").cloned().or_else(|| map.values().next().cloned())
}

fn cover_file_name(body: &serde_json::Value, manga_id: &str) -> Option<String> {
	let cover: Relationship = relationship(body, "cover_art")?;
	let file = cover.attributes.get("fileName").and_then(|value| value.as_str())?;
	Some(format!("{CDN_BASE}/covers/{manga_id}/{file}.512.jpg"))
}

fn manga_query(order: &str, page: u32) -> String {
	let offset = (page - 1) * LIST_LIMIT;
	let ratings: Vec<String> = CONTENT_RATINGS
		.iter()
		.map(|rating| format!("contentRating[]={rating}"))
		.collect();
	format!(
		"{API_BASE}/manga?limit={LIST_LIMIT}&offset={offset}&includes[]=cover_art&order[{order}]=desc&availableTranslatedLanguage[]=en&{}",
		ratings.join("&")
	)
}

fn parse_summary(body: &serde_json::Value) -> Vec<WorkSummary> {
	body["data"]
		.as_array()
		.map(|items| {
			items
				.iter()
				.filter_map(|item| {
					let id = item["id"].as_str()?;
					let attributes: MangaAttributes =
						serde_json::from_value(item["attributes"].clone()).ok()?;
					let title = attributes.title.get("en").cloned().or_else(|| {
						attributes
							.alt_titles
							.iter()
							.find_map(|alt| alt.get("en").cloned())
					})?;
					Some(WorkSummary {
						title,
						url: format!("{SITE_BASE}/title/{id}"),
						cover_url: cover_file_name(item, id),
					})
				})
				.collect()
		})
		.unwrap_or_default()
}

fn uuid_from_title_url(url: &str) -> Result<&str, SourceError> {
	url.rsplit('/').next().filter(|id| id.len() >= 32 && id.contains('-')).ok_or_else(|| {
		SourceError::NotFound
	})
}

struct MangaDex;

impl Guest for MangaDex {
	fn get_info() -> SourceInfo {
		SourceInfo {
			id: "manga_dex".to_string(),
			name: "MangaDex".to_string(),
			version: "1.0.0".to_string(),
			kind: WorkKind::Manga,
			icon_url: None,
			referer_url: Some(format!("{SITE_BASE}/")),
			base_url: Some(SITE_BASE.to_string()),
		}
	}

	fn search(query: String, page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 100 {
			return Ok(vec![]);
		}
		let encoded = urlencoding_encode(&query);
		let body = http_get_json(&format!(
			"{}&title={encoded}",
			manga_query("latestUploadedChapter", page)
		))?;
		Ok(parse_summary(&body))
	}

	fn latest(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 100 {
			return Ok(vec![]);
		}
		let body = http_get_json(&manga_query("latestUploadedChapter", page))?;
		Ok(parse_summary(&body))
	}

	fn trending(page: u32) -> Result<Vec<WorkSummary>, SourceError> {
		if page > 100 {
			return Ok(vec![]);
		}
		let body = http_get_json(&manga_query("followedCount", page))?;
		Ok(parse_summary(&body))
	}

	fn fetch_work(url: String) -> Result<WorkDetails, SourceError> {
		let manga_id = uuid_from_title_url(&url)?.to_string();
		let ratings: Vec<String> = CONTENT_RATINGS
			.iter()
			.map(|rating| format!("contentRating[]={rating}"))
			.collect();
		let body = http_get_json(&format!(
			"{API_BASE}/manga/{manga_id}?includes[]=cover_art&includes[]=author&includes[]=artist&{}",
			ratings.join("&")
		))?;

		let attributes: MangaAttributes = serde_json::from_value(body["data"]["attributes"].clone())
			.map_err(|error| SourceError::Parse(error.to_string()))?;
		let title = localized(&attributes.title)
			.or_else(|| attributes.alt_titles.iter().find_map(localized))
			.unwrap_or_else(|| url.clone());

		let mut authors = Vec::new();
		for kind in ["author", "artist"] {
			if let Some(names) = body["relationships"].as_array() {
				for entry in names.iter().filter(|entry| entry["type"] == kind) {
					if let Some(name) = entry["attributes"]["name"].as_str() {
						if !name.is_empty() && !authors.contains(&name.to_string()) {
							authors.push(name.to_string());
						}
					}
				}
			}
		}

		let status = attributes.status.as_deref().map(|status| match status {
			"completed" => "Completed".to_string(),
			"hiatus" => "Hiatus".to_string(),
			"cancelled" => "Cancelled".to_string(),
			_ => "Ongoing".to_string(),
		});

		let description = attributes.description.get("en").cloned();

		let mut genres = Vec::new();
		for tag in &attributes.tags {
			if tag.id == ONE_SHOT_TAG || tag.id == ANTHOLOGY_TAG {
				continue;
			}
			genres.push(tag_name(&tag.id).to_string());
		}

		let chapters = feed_chapters(&manga_id)?;

		Ok(WorkDetails {
			title,
			url,
			cover_url: cover_file_name(&body["data"], &manga_id),
			alternative_names: vec![],
			authors,
			artists: vec![],
			status,
			release_date: attributes.year.map(|year| year.to_string()),
			description,
			genres,
			chapters,
			content_html: None,
		})
	}

	fn fetch_chapter(url: String) -> Result<Vec<String>, SourceError> {
		let chapter_id = uuid_from_title_url(&url)?.trim().to_string();
		let at_home: AtHome = http_get_json(&format!("{API_BASE}/at-home/server/{chapter_id}?forcePort443=true"))
			.and_then(|body| {
				serde_json::from_value(body).map_err(|error| SourceError::Parse(error.to_string()))
			})?;
		if at_home.base_url.is_empty() {
			return Err(SourceError::Network("at-home returned no host".into()));
		}
		Ok(at_home
			.chapter
			.data
			.into_iter()
			.map(|page| format!("{}/data/{}/{}", at_home.base_url, at_home.chapter.hash, page))
			.collect())
	}
}

const ONE_SHOT_TAG: &str = "0234a31e-a729-4e28-9d6a-3f87c4966b9e";
const ANTHOLOGY_TAG: &str = "51d83883-4103-437c-b4b1-731cb73d786c";

fn tag_name(tag_id: &str) -> &'static str {
	match tag_id {
		"391b0423-d847-456f-aff0-8b0cfc03038b" => "Action",
		"aafb99c1-7f60-43fa-b75f-fc9502ce29c7" => "Horror",
		"5920b825-4181-4a17-be10-99577219b01e" => "Drama",
		"f5ba408b-0e6e-48ca-b089-795f5b69594e" => "Sports",
		"cdccad85-1493-4706-95d0-450b14f3fa66" => "Cooking",
		"cde1bc03-e7ac-400b-af9b-11dc72446c34" => "Combat",
		"256c8bd9-4904-4360-bf4f-508a76d67183" => "Sci-Fi",
		"cdc58593-87dd-415e-bbc0-2ec27bf404cc" => "Fantasy",
		"81c836c9-914a-4eca-981a-560dad663e73" => "Gag Humor",
		"ddefd648-5142-468e-b6f0-422164300946" => "Award Winning",
		"ee968100-4171-4968-b27b-10138d2a4c74" => "Martial Arts",
		"df33b061-20a3-4d84-a48f-c5cff5f8c102" => "Video Games",
		"f8f62992-7cec-4a74-9140-049246a24079" => "Web Comic",
		"a1f53773-ce69-4d54-8bd0-2b520120bee6" => "Adaptation",
		"ace04997-f6bd-436e-b261-779182194d3d" => "Isekai",
		"acc803a4-c95a-4c22-86fc-eb6b582d82a2" => "Superhero",
		"320831a8-4026-470b-94f6-8354840acf6f" => "Iyashikei",
		"07251805-a27e-4d59-b488-f0bccfb3aefd" => "Office Workers",
		"0a39b5a1-b235-4886-a747-1d05d216532d" => "Athletes",
		"ea2bc92d-1afe-4bc2-87ae-35013477a28b" => "Time Travel",
		"292e862b-2d17-4062-90a2-0356aa4cdbb5" => "Survival",
		"489dd859-9b61-4b37-87fa-f45ead0c78e7" => "Cross-Dressing",
		"85daba54-a71c-4554-8a28-9901a8b0afad" => "Mafia",
		"92d6d951-ca5e-429c-ac79-d97164cd140c" => "Post-Apocalyptic",
		"9ab8f526-7f3b-413f-b6da-5d33a68918e4" => "Villainess",
		"97893a4c-12af-4dac-b6be-0dffb0020025" => "Delinquents",
		"b9af330a-a069-4838-a9df-adf2f6dc31a5" => "Reverse Harem",
		"5fff9db5-7e58-444a-b080-7f9c85a26e49" => "Music",
		"69964a91-5ca9-4edf-b71c-ade55903073d" => "Cultivation",
		"fad12b5e-68ba-460e-b9ff-edbcecfafd0e" => "Genderswap",
		"3de8c15d-9dff-4f8b-95b9-feefd1c722a2" => "Loli",
		"a7065a7c-7eae-4474-a002-ced584633275" => "Shota",
		"2bd2e0d0-e35b-41c6-9060-962a7b1bbca8" => "Ghosts",
		"891cf779-9f88-47b7-abb2-c725e346ef2b" => "Police",
		"5b7de119-cfd6-42ec-8a72-676765d62b09" => "Traditional Games",
		"8d2dc5fe-3a43-4d6b-9a57-7dfc5a4fc5d1" => "Zombies",
		"be78d95d-c4b5-4634-9d60-d0a6b6a4d3d2" => "Memory Loss",
		"2dd8d4df-9b83-4f56-a130-e7a35bcd6766" => "Idols (Female)",
		"7b2ce280-79ef-4c09-9b58-198930885421" => "Idols (Male)",
		"e64f6742-c834-471d-8d72-dd51fc74e63c" => "Educational",
		"3e2b8dae-350e-4ef8-9f3d-7e8cd574e43a" => "Military",
		"ac72833b-f410-4e05-886c-64993eeff942" => "Ninja",
		"4d32cc48-9f00-4cca-9b5a-a839f0764984" => "Comedy",
		"cdad7e68-1419-41dd-bdce-27753074a640" => "Ecchi",
		"423e2eae-a7f2-41a8-a77b-eccc094a00ff" => "Romance",
		"b13b2a48-c720-44a9-9c77-39c997537f43" => "Supernatural",
		"caaa44eb-cd40-4177-b930-79d3ef2afe87" => "Medical",
		"eabc5b4c-6aff-42f3-b657-3e90cbd00b75" => "Thriller",
		"b9fc3a18-e470-487c-8133-f77e2f0f6ad0" => "Incest",
		"0bc34acb-738f-4a49-82b6-9db49818d6f9" => "Boxing",
		"33771934-028e-4cb3-8744-691e866aacf9" => "Survival",
		"87cc87de-a736-4fd8-b6d3-67a6ee269c1d" => "Campus Life",
		"c8cbe35b-1b2b-4ac3-b08c-232111bdcaf7" => "Historical",
		"0234a31e-a729-4e28-9d6a-3f87c4966b9e" => "Oneshot",
		"51d83883-4103-437c-b4b1-731cb73d786c" => "Anthology",
		"3b60b75c-a2d7-4860-ab56-05f391bb889c" => "Psychological",
		"ea2a92de-7deb-406b-8b39-36f6e6e7e0b5" => "Magic",
		"5ca48f85-080a-4c8a-b157-10a71553cf7f" => "Gyaru",
		"9438db5a-7e2a-4ac0-b39e-ea12346900ae" => "Gore",
		"631ef4f4-362c-4fd3-8b24-fefeefdd3554" => "Mecha",
		"e5301a23-eb50-4c9c-b3f7-4a7e02ae0f97" => "Sexual Violence",
		"1bc23b15-d4a3-4efc-a1a6-657cbd6555d0" => "Love triangles",
		"0c92ef58-2ba9-405b-a843-204f21ccdcb6" => "Reincarnation",
		"2d1f5d56-a1e5-4d0d-a961-219b8f31ed9e" => "Otaku Culture",
		"da2d33ca-3dd4-4062-9d3c-2ec1bae0c5ee" => "Collective Volume",
		"7dcde953-a6f8-4551-9619-3b2dca6b262d" => "Tragedy",
		"b1e97889-25b4-4258-b28b-cd7f4a283ea3" => "Animals",
		"7065a305-b63c-4b28-965c-6967e08c8d3e" => "Archery",
		"6dd003f7-11f6-4b53-9ce7-3566588206c7" => "Chess",
		"0b39104a-4d94-4e64-923f-bdcf5cfcb27a" => "Fight Club",
		"54828dfd-57c3-436c-b607-3d758a55219c" => "Virtual Reality",
		"e19dff06-8112-4181-9d3c-3d6f2325b98f" => "Photography",
		"51dc5d9c-5739-4a94-a372-e9422e195a84" => "Harem",
		_ => "",
	}
}

fn feed_chapters(manga_id: &str) -> Result<Vec<Chapter>, SourceError> {
	let mut uploads: Vec<(ChapterAttributes, String, Option<String>)> = Vec::new();
	let ratings: Vec<String> = CONTENT_RATINGS
		.iter()
		.map(|rating| format!("contentRating[]={rating}"))
		.collect();
	for page in 0..20u32 {
		let offset = page * FEED_LIMIT;
		let body = http_get_json(&format!(
			"{API_BASE}/manga/{manga_id}/feed?translatedLanguage[]=en&order[chapter]=asc&limit={FEED_LIMIT}&offset={offset}&includeExternalUrl=0&includeEmptyPages=0&includeFuturePublishAt=0&{}&includes[]=scanlation_group",
			ratings.join("&")
		))?;
		let entries = body["data"].as_array().cloned().unwrap_or_default();
		if entries.is_empty() {
			break;
		}
		let total = body["total"].as_u64().unwrap_or(0);
		for entry in &entries {
			let chapter: ChapterAttributes = serde_json::from_value(entry["attributes"].clone())
				.map_err(|error| SourceError::Parse(error.to_string()))?;
			if chapter.external_url.is_some() || chapter.pages == 0 {
				continue;
			}
			let group = entry["relationships"]
				.as_array()
				.and_then(|rels| rels.iter().find(|rel| rel["type"] == "scanlation_group"))
				.and_then(|rel| rel["attributes"]["name"].as_str())
				.map(str::to_string);
			let id = entry["id"].as_str().unwrap_or_default().to_string();
			uploads.push((chapter, id, group));
		}
		if (uploads.len() as u64) >= total || total == 0 {
			break;
		}
	}

	uploads.sort_by(|a, b| a.0.publish_at.cmp(&b.0.publish_at));
	let mut seen = std::collections::HashSet::new();
	let mut ordered: Vec<(String, ChapterAttributes, String, Option<String>)> = Vec::new();
	for (chapter, id, group) in uploads {
		let key = chapter.chapter.clone().unwrap_or_else(|| format!("oneshot:{}", chapter.title.clone().unwrap_or_default()));
		if !seen.insert(key) {
			continue;
		}
		let number = chapter.chapter.clone();
		ordered.push((number.unwrap_or_default(), chapter, id, group));
	}

	ordered.sort_by(|a, b| {
		let (an, bn) = (a.0.parse::<f64>(), b.0.parse::<f64>());
		match (an, bn) {
			(Ok(an), Ok(bn)) => an.partial_cmp(&bn).unwrap_or(std::cmp::Ordering::Equal),
			(Ok(_), _) => std::cmp::Ordering::Less,
			(_, Ok(_)) => std::cmp::Ordering::Greater,
			_ => std::cmp::Ordering::Equal,
		}
	});

	Ok(ordered
		.into_iter()
		.map(|(_, chapter, id, group)| Chapter {
			title: chapter_title(&chapter),
			url: format!("{SITE_BASE}/chapter/{id}"),
			date: Some(chapter.publish_at),
			scanlation_group: group,
		})
		.collect())
}

fn chapter_title(chapter: &ChapterAttributes) -> String {
	match (&chapter.chapter, &chapter.title) {
		(Some(number), Some(title)) if !title.is_empty() => format!("Ch. {number} - {title}"),
		(Some(number), _) => format!("Ch. {number}"),
		(_, Some(title)) if !title.is_empty() => title.clone(),
		_ => "Oneshot".to_string(),
	}
}

fn urlencoding_encode(text: &str) -> String {
	let mut out = String::new();
	for byte in text.bytes() {
		match byte {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
			other => out.push_str(&format!("%{other:02X}")),
		}
	}
	out
}

export!(MangaDex);
