local BASE = "https://mangakatana.com"
local parser = require("parser")

local function fetch(url)
	local response = http.get(url, {
		{ name = "Referer", value = BASE .. "/" },
		{ name = "Accept-Language", value = "en-US,en;q=0.9" },
	})
	if http.has_cloudflare_protection(response.body, response.status) then
		local solved = flaresolverr.get(url)
		if solved then
			return solved.body
		end
		fail("cloudflare", "challenge unsolved for " .. url, true)
	end
	if response.status ~= 200 then
		fail("network", "HTTP " .. response.status .. " for " .. url, true)
	end
	return response.body
end

local function absolute(url)
	if not url or url == "" then
		return nil
	end
	if url:match("^https?://") then
		return url
	end
	return BASE .. url
end

local function percent_encode(value)
	return (value:gsub("[^%w%-%_%.%~]", function(character)
		return string.format("%%%02X", string.byte(character))
	end))
end

local function attr_of(node, name)
	if not node then
		return nil
	end
	return html.attr(node, name)
end

local function text_of(node)
	if not node then
		return nil
	end
	local text = string.trim(html.text(node))
	return text ~= "" and text or nil
end

local function parse_list(doc, page)
	local works = {}
	for _, item in ipairs(html.find(doc, "#book_list > div.item")) do
		local link = html.find_one(item.html, "div.text > h3 > a") or html.find_one(item.html, "a")
		local url = absolute(attr_of(link, "href"))
		local title = text_of(link)
		local image = html.find_one(item.html, "img")
		if url and title then
			table.insert(works, {
				title = title,
				remote_url = url,
				cover_url = absolute(attr_of(image, "src") or attr_of(image, "data-src")),
			})
		end
	end
	if page > 1 and #works == 0 then
		return nil
	end
	return works
end

local function parse_genres(doc)
	local genres = {}
	for _, link in ipairs(html.find(doc, ".genres > a")) do
		local name = text_of(link)
		if name then
			table.insert(genres, name)
		end
	end
	return genres
end

local function parse_status(doc)
	local text = text_of(html.find_one(doc, ".value.status"))
	if not text then
		return nil
	end
	if text:find("Completed") then
		return "completed"
	end
	if text:find("Ongoing") then
		return "ongoing"
	end
	return text
end

local function parse_chapters(doc)
	local chapters = {}
	for _, row in ipairs(html.find(doc, "table.uk-table tbody tr")) do
		if html.find_one(row.html, ".chapter") then
			local link = html.find_one(row.html, "a")
			local url = absolute(attr_of(link, "href"))
			local title = text_of(link)
			if url and title then
				table.insert(chapters, {
					title = title,
					remote_url = url,
					date = text_of(html.find_one(row.html, ".update_time")),
				})
			end
		end
	end
	return chapters
end

local function parse_images(doc)
	local pages, err = parser.parse_images(doc)
	if not pages then
		fail("parse", err)
	end
	local resolved = {}
	for _, url in ipairs(pages) do
		local absolute_url = absolute(url)
		if absolute_url then
			table.insert(resolved, absolute_url)
		end
	end
	return resolved
end

function info()
	return {
		id = "manga_katana",
		name = "MangaKatana",
		version = "1.0.0",
		kind = "manga",
		icon_url = BASE .. "/assets/mangafire/favicon.svg",
		referer_url = BASE .. "/",
		base_url = BASE,
	}
end

function latest(page)
	return parse_list(fetch(BASE .. "/page/" .. page), page) or {}
end

function trending(page)
	return parse_list(fetch(BASE .. "/manga/page/" .. page), page) or {}
end

function search(query, page)
	local url = BASE .. "/page/" .. page .. "?search=" .. percent_encode(query)
	local doc = fetch(url)
	local heading = html.find_one(doc, "h1.heading")
	if heading then
		local found = parse_list(doc, 1) or {}
		if #found > 0 then
			return found
		end
		local cover = html.find_one(doc, "div.media div.cover img")
		return {
			{
				title = string.trim(html.text(heading)),
				remote_url = url,
				cover_url = absolute(attr_of(cover, "src")),
			},
		}
	end
	return parse_list(doc, page) or {}
end

function fetch_work(url)
	local doc = fetch(url)
	local title = text_of(html.find_one(doc, "h1.heading"))
	local cover = html.find_one(doc, "div.media div.cover img")
	local summary = text_of(html.find_one(doc, ".summary > p"))
	local alternative = text_of(html.find_one(doc, ".alt_name"))

	if not title then
		fail("parse", "work page has no heading: " .. url)
	end

	local authors = {}
	for _, node in ipairs(html.find(doc, ".author")) do
		local author = text_of(node)
		if author then
			table.insert(authors, author)
		end
	end

	local description = summary
	if alternative then
		description = (description or "") .. "\n\nAlternative title: " .. alternative
	end

	return {
		title = title,
		remote_url = url,
		cover_url = absolute(attr_of(cover, "src") or attr_of(cover, "data-src")),
		alternative_names = alternative and { alternative } or {},
		authors = authors,
		artists = {},
		status = parse_status(doc),
		release_date = nil,
		description = description,
		genres = parse_genres(doc),
		chapters = parse_chapters(doc),
		content_html = nil,
	}
end

function fetch_chapter(url)
	return parse_images(fetch(url))
end