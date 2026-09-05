-- SPDX-License-Identifier: MPL-2.0
-- Observation only: do not request a parse or change its scheduling.
local seen = {}
local function mark(name)
  if seen[name] then return end
  seen[name] = true
  local sec, usec = vim.uv.gettimeofday()
  local file = assert(io.open(assert(vim.env.RUNYTE_BENCH_EVENTS), 'a'))
  file:write(string.format('%s %.0f\n', name, sec * 1e9 + usec * 1e3))
  file:close()
end

vim.api.nvim_create_autocmd('BufReadPost', {
  callback = function(event)
    mark('file_loaded')
    if vim.fn.fnamemodify(event.file, ':e') ~= 'lua' then return end
    local parser = vim.treesitter.get_parser(event.buf, 'lua')
    local parse = parser.parse
    local function completed(trees)
      for _, tree in pairs(trees or {}) do
        local root = tree:root()
        local first = root:start()
        local last = root:end_()
        -- The fixtures have a final newline. A whole root must reach it.
        if first == 0 and last == vim.api.nvim_buf_line_count(event.buf)
            and not root:has_error() then
          mark('syntax_ready')
        end
      end
    end
    -- Observe the normal parse's return/callback, after the tree is installed.
    -- Preserve whether the caller requested synchronous or asynchronous work.
    function parser:parse(range, on_parse)
      if on_parse then
        return parse(self, range, function(err, trees)
          if not err then completed(trees) end
          on_parse(err, trees)
        end)
      end
      local trees = parse(self, range)
      completed(trees)
      return trees
    end
  end,
})
