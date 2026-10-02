local M = {}

function M.parse_images(doc)
	local name = doc:match("data%-src['\"]%s*,%s*(%w+)")
	if not name then
		return nil, "chapter page exposes no image array"
	end
	local list = doc:match("var " .. name .. "%s*=%s*%[(.-)%]")
	if not list then
		return nil, "image array " .. name .. " not found"
	end
	local pages = {}
	for url in list:gmatch("'([^']*)'") do
		if url ~= "" then
			table.insert(pages, url)
		end
	end
	return pages
end

return M