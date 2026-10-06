--!strict
--!native
-- jess's package bootstrap: what the host runs to inspect, pack, resolve, and activate packages.
-- It is not a module and nothing can require it. Every entry opens packages and resolves them
-- through @jess/package, which runs no package code; `activate` then hands the accepted plan to
-- the one private mount the host passes it, and only after that can `require` reach a package, or
-- the host run the plan's entrypoints.

type Entrypoint = { module: string, capabilities: { string } }
type Layer = { id: string, requires: { string }, exports: { string }, entrypoints: { Entrypoint } }
type Managed = { id: string, module: string, interface: string? }
type Target = {
  id: string,
  kind: 'application' | 'command',
  entry: string,
  capabilities: { string },
  command: string?,
}
type PlanEntrypoint = { module: string, package: string, path: string, layer: string }

type Package = {
  read manifest: {
    read jessApi: string,
    read defaultTarget: string?,
    read modules: { [string]: string },
    read dependencies: { [string]: string },
    read layers: { Layer },
    read managed: { [string]: Managed },
    read targets: { [string]: Target },
    read tests: { string },
  },
  read id: string,
  read version: string,
  read digest: string,
  read kind: string,
  read location: string,
  read files: { string },
  read read: (path: string) -> buffer?,
}

type Plan = {
  read modules: { { name: string, package: string, path: string } },
  read entrypoints: { PlanEntrypoint },
  read packages: { { read id: string } },
}

type PackageModule = {
  read open: (location: string) -> Package,
  read pack: (source: string, output: string) -> { [string]: unknown },
  read resolve: (packages: { Package }, request: { [string]: unknown }) -> Plan,
}

local package = (require :: (string) -> PackageModule) '@jess/package'

export type Mount = (
  modules: { { name: string, package: string, path: string } },
  entrypoints: { PlanEntrypoint },
  packages: { { id: string, version: string, digest: string, read: (path: string) -> buffer? } }
) -> ()

local function openAll(locations: { string }): { Package }
  local packages = {}

  for _, location in locations do
    table.insert(packages, package.open(location))
  end

  return packages
end

local function inspect(location: string)
  local opened = package.open(location)
  local manifest = opened.manifest
  local modules: { { name: string, path: string } } = {}
  local dependencies: { { id: string, requirement: string } } = {}
  local capabilities: { { capability: string, layer: string, entrypoint: string } } = {}
  local managed: { Managed } = {}
  local targets: { Target } = {}

  for name, path in manifest.modules do
    table.insert(modules, { name = name, path = path })
  end

  table.sort(
    modules,
    function(
      left: { name: string, path: string },
      right: { name: string, path: string }
    ): boolean
      return left.name < right.name
    end
  )

  for id, requirement in manifest.dependencies do
    table.insert(dependencies, { id = id, requirement = requirement })
  end

  table.sort(
    dependencies,
    function(
      left: { id: string, requirement: string },
      right: { id: string, requirement: string }
    ): boolean
      return left.id < right.id
    end
  )

  for _, declaration in manifest.managed do
    table.insert(managed, declaration)
  end

  table.sort(
    managed,
    function(left: Managed, right: Managed): boolean return left.id < right.id end
  )

  for _, target in manifest.targets do
    table.insert(targets, target)
  end

  table.sort(targets, function(left: Target, right: Target): boolean return left.id < right.id end)

  for _, layer in manifest.layers do
    for _, entrypoint in layer.entrypoints do
      for _, capability in entrypoint.capabilities do
        table.insert(
          capabilities,
          { capability = capability, layer = layer.id, entrypoint = entrypoint.module }
        )
      end
    end
  end

  return {
    id = opened.id,
    version = opened.version,
    digest = opened.digest,
    kind = opened.kind,
    location = opened.location,
    jessApi = manifest.jessApi,
    defaultTarget = manifest.defaultTarget,
    files = #opened.files,
    modules = modules,
    layers = manifest.layers,
    managed = managed,
    targets = targets,
    dependencies = dependencies,
    capabilities = capabilities,
    tests = manifest.tests,
  }
end

local function graph(locations: { string }, layers: { string }, hostModules: { string }): Plan
  return package.resolve(openAll(locations), { layers = layers, hostModules = hostModules })
end

local function activate(
  mount: Mount,
  locations: { string },
  layers: { string },
  hostModules: { string },
  grant: { string }?
): Plan
  local packages = openAll(locations)
  local plan =
    package.resolve(packages, { layers = layers, hostModules = hostModules, grant = grant })
  local byId: { [string]: Package } = {}
  local mounted: { { id: string, version: string, digest: string, read: (path: string) -> buffer? } } =
    {}

  for _, opened in packages do
    byId[opened.id] = opened
  end

  for _, selected in plan.packages do
    local opened = byId[selected.id]

    table.insert(mounted, {
      id = opened.id,
      version = opened.version,
      digest = opened.digest,
      read = opened.read,
    })
  end

  mount(plan.modules, plan.entrypoints, mounted)

  return plan
end

return table.freeze {
  inspect = inspect,
  pack = package.pack,
  graph = graph,
  activate = activate,
}
