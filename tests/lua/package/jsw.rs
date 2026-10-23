//! Packed workspaces: what `pack` freezes, that the frozen plan is the workspace's, what a run
//! needs (the one file), what opening one refuses, and where an executable carries one.

use std::path::{Path, PathBuf};

use jess::lua::image::{self, Image};
use jess::lua::package::TargetRun;
use jess::lua::workspace::{self, Locate, RunRequest};

use super::{common, library, text, write};

/// A workspace of two Package Sources, `org.example.app` and `org.example.lib`, with
/// `org.example.base` pinned as a JPK: what a run of the app's `hello` target resolves over.
fn workspace(root: &Path) -> PathBuf {
    let folder = root.join("workspace");
    library(&root.join("vendor/base"), "base", "", "return { name = 'base' }\n");
    library(
        &folder.join("lib"),
        "lib",
        "[dependencies]\n\"org.example.base\" = \"1\"\n",
        "return { name = 'lib on ' .. require('@base').name }\n",
    );
    write(
        &folder,
        &[
            (
                "jess-workspace.toml",
                "manifest_version = 1\n\n[workspace]\nmembers = [\"app\", \"lib\"]\n",
            ),
            (
                "app/jess.toml",
                "manifest_version = 1\n[package]\nid = \"org.example.app\"\nversion = \"1.0.0\"\njess_api = \"1\"\n\
                 default_target = \"hello\"\ninclude = [\"src\"]\n[dependencies]\n\"org.example.lib\" = \"1\"\n\
                 [targets.hello]\nkind = \"command\"\ncommand = \"hello\"\nentry = \"src/main.jsl\"\ncapabilities = []\n",
            ),
            ("app/src/main.jsl", "return 'hello from ' .. require('@lib').name, ...\n"),
        ],
    );
    let jpk = root.join("base.jpk");
    jess::lua::package::pack(&common::runtime(), &text(&root.join("vendor/base")), &text(&jpk)).unwrap();
    let request = workspace::SyncRequest {
        at: Locate::Root(folder.clone()),
        artifacts: vec![text(&jpk)],
        paths: Vec::new(),
        locked: false,
        offline: true,
        indexes: Vec::new(),
        publishers: Vec::new(),
    };
    workspace::sync(&common::runtime(), &request).unwrap_or_else(|error| panic!("{error}"));
    folder
}

fn pack(folder: &Path, output: &Path) -> workspace::Packed {
    workspace::pack(&common::runtime(), &Locate::Root(folder.to_path_buf()), output, None)
        .unwrap_or_else(|error| panic!("{error}"))
}

fn packed(path: &Path) -> Locate {
    Locate::Packed(Image::open(path).unwrap())
}

fn run(at: Locate, target: Option<&str>, args: &[&str]) -> Result<TargetRun, String> {
    let request = RunRequest {
        at,
        target: target.map(ToOwned::to_owned),
        cwd: std::env::temp_dir(),
        grant: None,
        args: args.iter().map(ToString::to_string).collect(),
    };
    workspace::run_target(&jess::lua::Options::default(), &request).map_err(|error| error.to_string())
}

fn strings(run: &TargetRun) -> Vec<String> {
    run.values
        .iter()
        .map(|value| {
            run.runtime
                .stack()
                .with_frame(|frame| value.push_to(frame)?.read::<String>())
                .unwrap()
        })
        .collect()
}

/// A packed workspace holds each exported target's whole plan and nothing else, the same bytes
/// every time, and runs from the one file with the workspace gone, resolving exactly as the
/// workspace did.
#[test]
fn a_packed_workspace_is_the_workspace_s_plan_in_one_file() {
    let temp = tempfile::tempdir().unwrap();
    let folder = workspace(temp.path());
    let output = temp.path().join("app.jsw");
    let report = pack(&folder, &output);
    assert_eq!(report.default.as_deref(), Some("org.example.app:hello"));
    assert_eq!(report.targets, ["org.example.app:hello"]);
    assert_eq!(
        report.packages.iter().map(|package| package.id.as_str()).collect::<Vec<_>>(),
        ["org.example.app", "org.example.base", "org.example.lib"]
    );
    let again = temp.path().join("again.jsw");
    pack(&folder, &again);
    assert_eq!(std::fs::read(&output).unwrap(), std::fs::read(&again).unwrap());

    let runtime = common::runtime();
    let at = packed(&output);
    let cwd = std::env::temp_dir();
    let from_workspace = workspace::resolve_target(&runtime, &Locate::Root(folder.clone()), Some("hello"), &cwd).unwrap();
    let from_image = workspace::resolve_target(&runtime, &at, None, &cwd).unwrap();
    let identity = |resolved: &workspace::ResolvedTarget| {
        let packages = resolved
            .plan
            .packages
            .iter()
            .map(|package| {
                (
                    package.id.clone(),
                    package.version.clone(),
                    package.digest.clone(),
                    package.dependencies.clone(),
                )
            })
            .collect::<Vec<_>>();
        let modules = resolved
            .plan
            .modules
            .iter()
            .map(|module| (module.name.clone(), module.package.clone(), module.path.clone()))
            .collect::<Vec<_>>();
        (packages, modules, resolved.target.capabilities.clone())
    };
    assert_eq!(identity(&from_workspace), identity(&from_image));
    assert!(
        from_image
            .plan
            .packages
            .iter()
            .all(|package| package.kind == "jpk" && package.location.starts_with(&format!("{}!/packages/", text(&output)))),
        "{from_image:#?}"
    );

    std::fs::remove_dir_all(&folder).unwrap();
    std::fs::remove_dir_all(temp.path().join("vendor")).unwrap();
    std::fs::remove_file(temp.path().join("base.jpk")).unwrap();
    let ran = run(at, None, &["and more"]).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(strings(&ran), ["hello from lib on base", "and more"]);

    let opened = workspace::open(&runtime, &packed(&output)).unwrap();
    assert_eq!(opened.kind, "packed");
    assert_eq!(opened.members.len(), 1);
    assert_eq!(opened.members[0].default_target.as_deref(), Some("hello"));
    assert_eq!(opened.lock.packages.len(), 3);
    for doing in [
        workspace::test(&jess::lua::Options::default(), &packed(&output), &[], temp.path()).map(|_| ()),
        workspace::pack(&runtime, &packed(&output), &again, None).map(|_| ()),
    ] {
        let refused = doing.unwrap_err().to_string();
        assert!(refused.contains("is a packed workspace, which is sealed"), "{refused}");
    }
}

/// `[pack]` names the targets a packed workspace exports and its default, by the names `run`
/// takes; a lock that isn't fresh is never packed.
#[test]
fn a_workspace_file_says_what_is_packed() {
    let temp = tempfile::tempdir().unwrap();
    let folder = workspace(temp.path());
    let file = folder.join("jess-workspace.toml");
    let listed = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        format!("{listed}\n[pack]\ntargets = [\"hello\"]\ndefault = \"org.example.app\"\n"),
    )
    .unwrap();
    let report = pack(&folder, &temp.path().join("app.jsw"));
    assert_eq!(report.default.as_deref(), Some("org.example.app:hello"));

    std::fs::write(&file, format!("{listed}\n[pack]\ntargets = [\"nothing\"]\n")).unwrap();
    let unnamed = workspace::pack(
        &common::runtime(),
        &Locate::Root(folder.clone()),
        &temp.path().join("x.jsw"),
        None,
    )
    .unwrap_err()
    .to_string();
    assert!(unnamed.contains("nothing in the workspace is named 'nothing'"), "{unnamed}");

    std::fs::write(&file, &listed).unwrap();
    std::fs::remove_file(folder.join("jess.lock")).unwrap();
    let stale = workspace::pack(&common::runtime(), &Locate::Root(folder), &temp.path().join("x.jsw"), None)
        .unwrap_err()
        .to_string();
    assert!(stale.contains("jess.lock is out of date"), "{stale}");
    assert!(!temp.path().join("x.jsw").exists());
}

/// Rewrites the packed workspace at `from` into `to` with `change`, a Luau chunk over `files`
/// (each `{ path, data, store }`, everything stored) and `index`, `jsw.toml`'s text, which is
/// written back. `extra` is a file the chunk can read, as the global `extra`.
fn rewritten(from: &Path, to: &Path, change: &str, extra: Option<&Path>) {
    let runtime = common::filesystem_runtime();
    common::set_global(&runtime, "from", text(from).as_str());
    common::set_global(&runtime, "to", text(to).as_str());
    if let Some(extra) = extra {
        common::set_global(&runtime, "extra", text(extra).as_str());
    }
    common::run(
        &runtime,
        &format!(
            "local jpk, fs = require '@jess/jpk', require '@dream/fs'\n\
             local archive = jpk.open(assert(fs.readFile(from)))\n\
             local files = {{}}\n\
             for _, entry in archive.entries do\n  \
               if entry.path ~= 'jsw.toml' then\n    \
                 table.insert(files, {{ path = entry.path, data = archive.read(entry), store = true }})\n  \
               end\n\
             end\n\
             local index = buffer.tostring(archive.read 'jsw.toml')\n\
             {change}\n\
             table.insert(files, {{ path = 'jsw.toml', data = buffer.fromstring(index), store = true }})\n\
             assert(fs.writeFile(to, jpk.write(files), {{ atomic = true }}))"
        ),
    );
}

/// A packed workspace is opened as untrusted input: an index that disagrees with what is held, a
/// package that isn't the one pinned, anything held that isn't listed, and a jess it wasn't packed
/// for are each refused, before anything a package holds runs.
#[test]
fn a_packed_workspace_that_does_not_add_up_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let folder = workspace(temp.path());
    let good = temp.path().join("app.jsw");
    let report = pack(&folder, &good);
    let base = &report
        .packages
        .iter()
        .find(|package| package.id == "org.example.base")
        .unwrap()
        .digest;
    let bad = temp.path().join("bad.jsw");
    let refusal = |change: &str| -> String {
        rewritten(&good, &bad, change, None);
        let opened = Image::open(&bad).map_err(|error| error.to_string());
        match opened.and_then(|image| run(Locate::Packed(image), None, &[]).map(|_| ())) {
            Ok(()) => panic!("expected a refusal for {change}"),
            Err(error) => error,
        }
    };

    let repinned = refusal(&format!("index = string.gsub(index, '{base}', string.rep('a', 64))"));
    assert!(
        repinned.contains(&format!("has digest {base}, not the {} it is pinned to", "a".repeat(64))),
        "{repinned}"
    );
    let stray = refusal("table.insert(files, { path = 'notes.txt', data = buffer.fromstring 'hi' })");
    assert!(stray.contains("holds 'notes.txt', which its index doesn't list"), "{stray}");
    let missing = refusal(
        "for at, file in files do\n  \
           if file.path == 'packages/org.example.base-1.0.0.jpk' then table.remove(files, at) break end\n\
         end",
    );
    assert!(
        missing.contains("lists org.example.base 1.0.0, which it doesn't hold"),
        "{missing}"
    );
    let swapped = refusal(
        "local lib\n\
         for _, file in files do if file.path == 'packages/org.example.lib-1.0.0.jpk' then lib = file.data end end\n\
         for _, file in files do if file.path == 'packages/org.example.base-1.0.0.jpk' then file.data = lib end end",
    );
    assert!(
        swapped.contains("is org.example.lib 1.0.0, not the org.example.base 1.0.0 its index lists"),
        "{swapped}"
    );
    let future = refusal("index = string.gsub(index, 'format_version = 1', 'format_version = 2')");
    assert!(
        future.contains("is format_version 2, and this jess reads format_version 1"),
        "{future}"
    );
    let elsewhere = refusal("index = string.gsub(index, 'builtin = \"[^\"]*\"', 'builtin = \"9.0.0\"')");
    assert!(
        elsewhere.contains("was packed for jess's built-in package 9.0.0"),
        "{elsewhere}"
    );
    let undeclared = refusal("index = string.gsub(index, 'id = \"hello\"', 'id = \"goodbye\"')");
    assert!(
        undeclared.contains("exports org.example.app:goodbye, which none of its packages declares"),
        "{undeclared}"
    );

    // A package claiming the built-in package's id is refused when opened, as one on disk is.
    let forged = temp.path().join("forged");
    write(
        &forged,
        &[
            (
                "jess.toml",
                "manifest_version = 1\n[package]\nid = \"org.dreamweave.jess\"\nversion = \"0.1.0\"\njess_api = \"1\"\n\
                 include = [\"src\"]\n[targets.evil]\nkind = \"command\"\nentry = \"src/evil.jsl\"\n",
            ),
            ("src/evil.jsl", "error('this must never run')\n"),
        ],
    );
    let forged_jpk = temp.path().join("forged.jpk");
    jess::lua::package::pack(&common::runtime(), &text(&forged), &text(&forged_jpk)).unwrap();
    // Refused before its digest is ever compared, so any digest will do.
    let digest = "b".repeat(64);
    rewritten(
        &good,
        &bad,
        &format!(
            "table.insert(files, {{ path = 'packages/org.dreamweave.jess-0.1.0.jpk', data = assert(fs.readFile(extra)), store = true }})\n\
             index ..= '\\n[[target]]\\npackage = \"org.dreamweave.jess\"\\nid = \"evil\"\\n\\n[[package]]\\nid = \"org.dreamweave.jess\"\\nversion = \"0.1.0\"\\ndigest = \"{digest}\"\\n'"
        ),
        Some(&forged_jpk),
    );
    let impostor = run(packed(&bad), Some("org.dreamweave.jess:evil"), &[]).unwrap_err();
    assert!(impostor.contains("says it is jess's built-in package"), "{impostor}");
}

/// The bytes of an executable carrying `payload` after `runtime`, as [`image::standalone`] writes it.
fn carrying(runtime: &[u8], payload: &[u8]) -> Vec<u8> {
    image::standalone(runtime, payload).unwrap_or_else(|problem| panic!("{problem}"))
}

/// A PE32+ header with room after it, signed when `certificate` is given: its certificate table's
/// file offset and size, which signing appends at the end.
fn portable_executable(certificate: Option<(u32, u32)>) -> Vec<u8> {
    portable_executable_at(0x80, certificate)
}

/// [`portable_executable`] with its PE headers at `at`, after a DOS stub that long.
fn portable_executable_at(at: usize, certificate: Option<(u32, u32)>) -> Vec<u8> {
    let mut bytes = vec![0; at + 0x180];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&u32::try_from(at).unwrap().to_le_bytes());
    bytes[at..at + 4].copy_from_slice(b"PE\0\0");
    bytes[at + 24..at + 26].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes[at + 24 + 108..at + 24 + 112].copy_from_slice(&16u32.to_le_bytes());
    if let Some((offset, size)) = certificate {
        let directory = at + 24 + 112 + 4 * 8;
        bytes[directory..directory + 4].copy_from_slice(&offset.to_le_bytes());
        bytes[directory + 4..directory + 8].copy_from_slice(&size.to_le_bytes());
    }
    // A runtime that looks for a trailer holds its magic.
    bytes[0x40..0x50].copy_from_slice(image::MAGIC);
    bytes
}

/// An executable with its PE headers at `at` carries a packed workspace after itself, behind a
/// trailer; signing it afterwards keeps it readable, and bytes added after the signature are
/// refused, never read.
fn carries_inside_its_signature(at: usize) {
    let temp = tempfile::tempdir().unwrap();
    let payload = b"the packed workspace".to_vec();
    let runtime = portable_executable_at(at, None);
    let unsigned = carrying(&runtime, &payload);
    let path = temp.path().join("app.exe");
    std::fs::write(&path, &unsigned).unwrap();
    let found = Image::carried(&path).unwrap().unwrap();
    assert_eq!((found.offset, found.length), (runtime.len() as u64, payload.len() as u64));

    // Signing pads to 8 bytes and appends the certificate table, which the headers then name.
    let mut signed = unsigned;
    signed.resize(signed.len().next_multiple_of(8), 0);
    let table = u32::try_from(signed.len()).unwrap();
    signed.extend_from_slice(&[0x5a; 64]);
    let header = portable_executable_at(at, Some((table, 64)));
    signed[..header.len()].copy_from_slice(&header);
    std::fs::write(&path, &signed).unwrap();
    let found = Image::carried(&path).unwrap().unwrap();
    assert_eq!((found.offset, found.length), (runtime.len() as u64, payload.len() as u64));

    let mut appended = signed.clone();
    appended.extend_from_slice(&carrying(&runtime, b"something else")[runtime.len()..]);
    std::fs::write(&path, &appended).unwrap();
    let refused = Image::carried(&path).unwrap_err().to_string();
    assert!(refused.contains("has bytes after its signature"), "{refused}");

    std::fs::write(&path, &runtime).unwrap();
    assert_eq!(Image::carried(&path).unwrap(), None);
    assert_eq!(Image::open(&path).unwrap().length, runtime.len() as u64);
}

/// An executable carries a packed workspace after itself, and inside a signature added after,
/// wherever its DOS stub puts its PE headers, past the first page included. A file that starts as
/// a Windows executable but whose PE headers don't read is refused, never taken as unsigned, and a
/// runtime that can't carry one is refused before anything is written: every universal and
/// non-native Mach-O among them.
#[test]
fn an_executable_carries_a_packed_workspace_inside_its_signature() {
    carries_inside_its_signature(0x80);
    carries_inside_its_signature(0x1200);

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("app.exe");
    let payload = b"the packed workspace".to_vec();
    let (mut lost, mut unreadable) = (portable_executable(None), portable_executable(None));
    lost[0x3c..0x40].copy_from_slice(&0x10_0000u32.to_le_bytes());
    unreadable[0x80 + 24] = 0;
    for (broken, why) in [(lost, "has no PE headers"), (unreadable, "neither PE32 nor PE32+")] {
        std::fs::write(&path, &broken).unwrap();
        let refused = Image::carried(&path).unwrap_err().to_string();
        assert!(refused.contains(why), "{refused}");
        let problem = image::standalone(&broken, &payload).unwrap_err();
        assert!(problem.contains(why), "{problem}");
    }

    let mach_o = [
        [0xca, 0xfe, 0xba, 0xbe],
        [0xca, 0xfe, 0xba, 0xbf],
        [0xbe, 0xba, 0xfe, 0xca],
        [0xbf, 0xba, 0xfe, 0xca],
        [0xfe, 0xed, 0xfa, 0xce],
        [0xfe, 0xed, 0xfa, 0xcf],
        [0xce, 0xfa, 0xed, 0xfe],
    ]
    .map(|magic| (magic.repeat(8), "is a universal, 32-bit, or big-endian"));
    let unsigned = carrying(&portable_executable(None), &payload);
    for (unfit, why) in mach_o.into_iter().chain([
        (portable_executable(Some((512, 8))), "is already signed"),
        (unsigned, "already carries a packed workspace"),
        (vec![0; 64], "predates standalone executables"),
    ]) {
        let problem = image::standalone(&unfit, &payload).unwrap_err();
        assert!(problem.contains(why), "{problem}");
    }
}

const PAGE: usize = 0x4000;

/// A Mach-O load command: its kind, then `body`, its size counted in.
fn command(kind: u32, body: &[u8]) -> Vec<u8> {
    let mut bytes = kind.to_le_bytes().to_vec();
    bytes.extend_from_slice(&u32::try_from(body.len() + 8).unwrap().to_le_bytes());
    bytes.extend_from_slice(body);
    bytes
}

fn name16(name: &str) -> [u8; 16] {
    let mut field = [0; 16];
    field[..name.len()].copy_from_slice(name.as_bytes());
    field
}

/// An `LC_SEGMENT_64` named `name`, with one section `section` filling it when given.
fn segment(name: &str, vmaddr: u64, fileoff: u64, filesize: u64, section: Option<(&str, u64)>) -> Vec<u8> {
    let mut body = name16(name).to_vec();
    for value in [vmaddr, filesize, fileoff, filesize] {
        body.extend_from_slice(&value.to_le_bytes());
    }
    body.extend_from_slice(&[3, 0, 0, 0, 3, 0, 0, 0]);
    body.extend_from_slice(&u32::from(section.is_some()).to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    if let Some((section, size)) = section {
        body.extend_from_slice(&name16(section));
        body.extend_from_slice(&name16(name));
        body.extend_from_slice(&vmaddr.to_le_bytes());
        body.extend_from_slice(&size.to_le_bytes());
        body.extend_from_slice(&u32::try_from(fileoff).unwrap().to_le_bytes());
        body.extend_from_slice(&[0; 28]);
    }
    command(0x19, &body)
}

/// A thin arm64 Mach-O laid out as a macOS jess is: `__TEXT`, then the reserved `__JESS,__jsw`
/// holding the placeholder (when `reserved`), then `__LINKEDIT` with a symbol table and, when
/// `signed`, an ad hoc signature for the identifier `jess-test`.
fn mach_o(reserved: bool, signed: bool) -> Vec<u8> {
    let linkedit = if reserved { 2 * PAGE } else { PAGE };
    let (symbols, strings) = (b"symbols-symbols!".as_slice(), b"strings\0".as_slice());
    let mut identity = Vec::new();
    for word in [0xfade_0cc0u32, 0, 1, 0, 20, 0xfade_0c02, 0, 0x20400, 2, 0, 88] {
        identity.extend_from_slice(&word.to_be_bytes());
    }
    identity.extend_from_slice(&[0; 64]);
    identity.extend_from_slice(b"jess-test\0");
    let signature_at = (linkedit + symbols.len() + strings.len()).next_multiple_of(16);
    let end = if signed {
        signature_at + identity.len()
    } else {
        linkedit + symbols.len() + strings.len()
    };

    let mut commands = segment("__TEXT", 0x1_0000_0000, 0, PAGE as u64, None);
    if reserved {
        commands.extend(segment(
            "__JESS",
            0x1_0000_4000,
            PAGE as u64,
            PAGE as u64,
            Some(("__jsw", 16)),
        ));
    }
    let base = 0x1_0000_0000 + linkedit as u64;
    commands.extend(segment("__LINKEDIT", base, linkedit as u64, (end - linkedit) as u64, None));
    let mut symtab = Vec::new();
    for value in [linkedit, 1, linkedit + symbols.len(), strings.len()] {
        symtab.extend_from_slice(&u32::try_from(value).unwrap().to_le_bytes());
    }
    commands.extend(command(0x2, &symtab));
    let count = if signed { 4 } else { 3 } + u32::from(reserved);
    if signed {
        let mut body = u32::try_from(signature_at).unwrap().to_le_bytes().to_vec();
        body.extend_from_slice(&u32::try_from(identity.len()).unwrap().to_le_bytes());
        commands.extend(command(0x1d, &body));
    }

    let mut bytes = Vec::new();
    for word in [
        0xfeed_facfu32,
        0x0100_000c,
        0,
        2,
        count,
        u32::try_from(commands.len()).unwrap(),
        0x0020_0085,
        0,
    ] {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes.extend_from_slice(&commands);
    bytes.resize(PAGE, 0xc3);
    if reserved {
        bytes.extend_from_slice(image::MAGIC);
        bytes.resize(2 * PAGE, 0);
    }
    bytes.extend_from_slice(symbols);
    bytes.extend_from_slice(strings);
    if signed {
        bytes.resize(signature_at, 0);
        bytes.extend_from_slice(&identity);
    }
    bytes
}

fn le32(bytes: &[u8], at: usize) -> usize {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
}

fn be32(bytes: &[u8], at: usize) -> usize {
    u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
}

/// A macOS jess carries a packed workspace in the section it reserves: the payload in the section,
/// `__LINKEDIT` after it with every offset into it moved along, and a fresh ad hoc signature
/// hashing every page before it, so the result runs and `codesign` can sign it again.
#[test]
fn a_macos_executable_carries_a_packed_workspace_in_its_reserved_section() {
    use sha2::Digest;

    let temp = tempfile::tempdir().unwrap();
    let payload = (0..40_000u32).map(|index| (index % 251) as u8).collect::<Vec<_>>();
    let runtime = mach_o(true, true);
    let path = temp.path().join("jess");
    std::fs::write(&path, &runtime).unwrap();
    assert_eq!(Image::carried(&path).unwrap(), None, "the placeholder carries nothing");

    let carrying = image::standalone(&runtime, &payload).unwrap_or_else(|problem| panic!("{problem}"));
    assert_eq!(&carrying[..32][..4], &runtime[..4]);
    let grown = payload.len().div_ceil(PAGE) * PAGE;
    let linkedit = PAGE + grown;
    // Load commands, in order: __TEXT, __JESS and its section, __LINKEDIT, LC_SYMTAB, the signature.
    let (jess, section, linkedit_command, symtab, signature) = (32 + 72, 32 + 144, 32 + 224, 32 + 296, 32 + 320);
    assert_eq!(&carrying[PAGE..PAGE + payload.len()], &payload[..]);
    assert_eq!(le32(&carrying, section + 48), PAGE);
    assert_eq!(le32(&carrying, section + 40), payload.len());
    assert_eq!(le32(&carrying, jess + 48), grown);
    assert_eq!(le32(&carrying, linkedit_command + 40), linkedit);
    assert_eq!(le32(&carrying, linkedit_command + 24), 0x4000 + grown);
    assert_eq!(&carrying[le32(&carrying, symtab + 8)..][..16], b"symbols-symbols!");
    assert_eq!(&carrying[le32(&carrying, symtab + 16)..][..8], b"strings\0");

    let (dataoff, datasize) = (le32(&carrying, signature + 8), le32(&carrying, signature + 12));
    assert_eq!(dataoff + datasize, carrying.len());
    assert_eq!(le32(&carrying, linkedit_command + 48), carrying.len() - linkedit);
    let blob = &carrying[dataoff..];
    assert_eq!(be32(blob, 0), 0xfade_0cc0);
    let directory = &blob[be32(blob, 16)..];
    assert_eq!(be32(directory, 0), 0xfade_0c02);
    assert_eq!(be32(directory, 32), dataoff, "the code limit is the signature");
    let identifier = &directory[be32(directory, 20)..];
    assert_eq!(&identifier[..10], b"jess-test\0");
    let (hashes, slots) = (be32(directory, 16), be32(directory, 28));
    assert_eq!(slots, dataoff.div_ceil(4096));
    for (index, page) in carrying[..dataoff].chunks(4096).enumerate() {
        assert_eq!(
            &directory[hashes + index * 32..hashes + index * 32 + 32],
            &sha2::Sha256::digest(page)[..],
            "page {index}"
        );
    }

    std::fs::write(&path, &carrying).unwrap();
    let found = Image::carried(&path).unwrap().unwrap();
    assert_eq!((found.offset, found.length), (PAGE as u64, payload.len() as u64));

    let unsigned = image::standalone(&mach_o(true, false), &payload).unwrap();
    assert_eq!(&unsigned[unsigned.len() - 8..], b"strings\0", "nothing is signed that wasn't");
    for (unfit, why) in [
        (carrying, "already carries a packed workspace"),
        (mach_o(false, true), "predates standalone executables"),
    ] {
        let problem = image::standalone(&unfit, &payload).unwrap_err();
        assert!(problem.contains(why), "{problem}");
    }
}
