--!strict
--!native
-- @jess/log: structured logging for scripts and packages, over one native sink the host owns.

export type Fields = { [string]: string | number | boolean }
export type Record = {
  read level: string,
  read target: string,
  read message: string,
  read fields: Fields,
}

export type Logger = {
  read debug: (message: string, fields: Fields?) -> (),
  read info: (message: string, fields: Fields?) -> (),
  read warn: (message: string, fields: Fields?) -> (),
  read error: (message: string, fields: Fields?) -> (),
  read child: (fields: Fields) -> Logger,
}

return function(emit: (level: string, target: string, message: string, fields: Fields) -> ())
  -- Captures are per thread; `coroutine.running()` is nil on the main thread, keyed as `false`.
  local captures: { [thread | boolean]: { { Record } } } = {}

  local function merge(parent: Fields, fields: Fields?): Fields
    if not fields then return parent end

    local result = table.clone(parent)

    for key, value in fields do
      assert(type(key) == 'string', 'log field names must be strings')
      assert(
        type(value) == 'string' or type(value) == 'number' or type(value) == 'boolean',
        'log fields must be strings, numbers, or booleans'
      )

      if type(value) == 'number' then
        assert(value == value and math.abs(value) ~= math.huge, 'log fields must be finite')
      end

      result[key] = value
    end

    return table.freeze(result)
  end

  local function logger(target: string, context: Fields): Logger
    local function write(level: string, message: string, fields: Fields?)
      assert(type(message) == 'string', 'a log message must be a string')
      local merged = merge(context, fields)
      local active = captures[coroutine.running() or false]

      if active then
        local record: Record =
          table.freeze { level = level, target = target, message = message, fields = merged }

        for _, records in active do
          table.insert(records, record)
        end

        return
      end

      emit(level, target, message, merged)
    end

    return table.freeze {
      debug = function(message: string, fields: Fields?) write('debug', message, fields) end,
      info = function(message: string, fields: Fields?) write('info', message, fields) end,
      warn = function(message: string, fields: Fields?) write('warn', message, fields) end,
      error = function(message: string, fields: Fields?) write('error', message, fields) end,
      child = function(fields: Fields): Logger return logger(target, merge(context, fields)) end,
    }
  end

  local NoFields: Fields = table.freeze({} :: Fields)

  local function new(target: string, fields: Fields?): Logger
    assert(type(target) == 'string', 'a logger target must be a string')
    return logger(target, merge(NoFields, fields))
  end

  local default = new 'jess'

  local function capture(body: () -> ()): { Record }
    local records: { Record } = {}
    local thread = coroutine.running() or false
    local active = captures[thread] or {}
    captures[thread] = active
    table.insert(active, records)
    -- The body is the caller's code. If it fails, this capture still has to come off the stack,
    -- or every later record on this thread would vanish into it; the failure is raised again
    -- unchanged.
    local success, failure = pcall(body)
    table.remove(active)

    if #active == 0 then captures[thread] = nil end

    if not success then error(failure, 0) end

    return records
  end

  return table.freeze {
    new = new,
    debug = default.debug,
    info = default.info,
    warn = default.warn,
    error = default.error,
    capture = capture,
  }
end
