--!strict
local log, test = require '@jess/log', require '@jess/test'
local expect = test.expect
local suite = test.new()

suite.describe('the modules', function()
  suite.it('are the same object on every require', function()
    expect(require '@jess/log').toBe(log)
    expect(require '@jess/test').toBe(test)
  end)

  suite.it('are frozen, loggers included', function()
    expect(table.isfrozen(log)).toBe(true)
    expect(table.isfrozen(test)).toBe(true)
    expect(table.isfrozen(log.new('test').child { name = 'child' })).toBe(true)
    expect(table.isfrozen(expect(1))).toBe(true)
    expect(function()
      (log :: any).info = function() end
    end).toThrow 'readonly'

    expect(function()
      (test :: any).expect = function() end
    end).toThrow 'readonly'

    expect(function() (log.new 'test' :: any).warn = nil end).toThrow 'readonly'
  end)

  suite.it('keep the sink private and the namespace closed', function()
    expect(function() require '@jess/private-log-sink' end).toThrow 'module not found'
    expect(function() require '@jess/not-a-built-in' end).toThrow 'jess\'s own namespace'
    expect(rawget(_G :: any, 'emit')).toBeNil()
  end)
end)

suite.describe('log', function()
  suite.it('merges context without keeping the caller table', function()
    local context = { count = 1 }
    local logger = log.new('test', context).child { name = 'child' }
    context.count = 9
    local records = log.capture(function() logger.info('message', { count = 2 }) end)
    expect(records[1].fields).toEqual { count = 2, name = 'child' }
    expect(records[1].level).toBe 'info'
    expect(records[1].target).toBe 'test'
    expect(table.isfrozen(records[1])).toBe(true)
    expect(table.isfrozen(records[1].fields)).toBe(true)
  end)

  suite.it('rejects fields the sink could not carry', function()
    local logger = log.new 'test'
    expect(function() logger.info('bad', { count = math.huge }) end).toThrow 'finite'
    expect(function() logger.info('bad', { count = 0 / 0 }) end).toThrow 'finite'
    expect(function() logger.info('bad', { list = {} :: any }) end).toThrow 'strings, numbers, or booleans'
    expect(function() logger.info((1 :: any) :: string) end).toThrow 'must be a string'
  end)

  suite.it('captures everything logged in the body, nested captures included', function()
    local outer = log.capture(function()
      local inner = log.capture(function() log.warn 'inner' end)
      expect(#inner).toBe(1)
      expect(function()
        log.capture(function() error 'callback failed' end)
      end).toThrow 'callback failed'
      log.info 'outer'
    end)
    expect(#outer).toBe(2)
    expect(outer[1].message).toBe 'inner'
    expect(outer[2].message).toBe 'outer'
    expect(outer[2].target).toBe 'jess'
  end)
end)

suite.describe('test', function()
  suite.it('compares deeply, cycles included', function()
    local left: { [string]: unknown }, right: { [string]: unknown } = {}, {}
    left.self, right.self = left, right
    expect(left).toEqual(right)
    right.extra = true
    expect(function() expect(left).toEqual(right) end).toThrow 'to equal'
    expect({ 1, 2, { value = 3 } }).toContain { value = 3 }
    expect('Morrowind.esm').toContain '.esm'
    expect(function() expect({ 1 }).toContain(2) end).toThrow 'expected { 1 } to contain 2'
  end)

  suite.it('describes what differs', function()
    expect(function() expect({ name = 'a', 1 }).toEqual { name = 'b' } end).toThrow 'expected { 1, name = "a" } to equal { name = "b" }'
    expect(function() expect(nil).toBeTruthy() end).toThrow 'expected nil to be truthy true'
  end)

  suite.it('runs hooks around failed setup and keeps the logs', function()
    local nested = test.new()
    local cleaned = 0
    nested.describe('group', function()
      nested.beforeEach(function() error 'setup' end)
      nested.afterEach(function()
        cleaned += 1
        log.warn 'cleanup'
      end)
      nested.it('never entered', function() error 'body' end)
    end)
    nested.it('passes', function() log.info 'fine' end)
    local report = nested.run()
    expect({ report.passed, report.failed, cleaned }).toEqual { 1, 1, 1 }
    expect(report.tests[1].name).toBe 'group / never entered'
    expect(report.tests[1].error).toContain 'setup'
    expect(report.tests[1].logs[1].message).toBe 'cleanup'
    expect(report.tests[2].passed).toBe(true)
    expect(report.tests[2].logs[1].message).toBe 'fine'
  end)
end)

local report = suite.run()

for _, result in report.tests do
  assert(result.passed, result.error)
end

assert(report.failed == 0 and report.passed == 9, `{report.passed} passed, {report.failed} failed`)
