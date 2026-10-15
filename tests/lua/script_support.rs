//! `jess.fs`, TOML through `@jess/data/toml`, and `jess.regex`: what load-order scripts use besides
//! records.

use jess::lua::l3i::Runtime;

mod common;

use common::{run, set_global};

fn lua() -> Runtime {
    common::runtime()
}

/// A runtime granted exactly `capabilities`.
fn granted(capabilities: &[&str]) -> Runtime {
    let capabilities = capabilities.iter().map(ToString::to_string).collect::<Vec<_>>();
    jess::lua::runtime_with_capabilities(jess::lua::Options::default(), &capabilities)
        .unwrap_or_else(|error| panic!("{error}"))
}

/// `jess.fs` reaches the host's files only as far as the runtime's policy grants: nothing without
/// a capability, reads with `filesystem.read`, changes with `filesystem.write`. Taking a path
/// apart or joining one touches nothing and needs neither.
#[test]
fn files_need_the_capability_that_covers_them() {
    let directory = tempfile::tempdir().unwrap();
    let existing = directory.path().join("existing.txt");
    std::fs::write(&existing, "kept").unwrap();
    let reads = [
        "read",
        "readBuffer",
        "exists",
        "isDir",
        "isFile",
        "size",
        "stat",
        "modified",
        "list",
        "cwd",
    ];
    let writes = ["write", "replace", "copy", "remove", "createDir"];
    for (capabilities, may_read, may_write) in [
        (&[][..], false, false),
        (&["filesystem.read"][..], true, false),
        (&["filesystem.write"][..], false, true),
        (&["filesystem.read", "filesystem.write"][..], true, true),
    ] {
        let lua = granted(capabilities);
        set_global(&lua, "dir", &directory.path().display().to_string());
        set_global(&lua, "existing", &existing.display().to_string());
        for (names, capability, allowed) in [
            (&reads[..], "filesystem.read", may_read),
            (&writes[..], "filesystem.write", may_write),
        ] {
            for name in names {
                set_global(&lua, "name", *name);
                set_global(&lua, "capability", capability);
                set_global(&lua, "allowed", &allowed);
                run(
                    &lua,
                    r#"
                    local scratch = jess.fs.join(dir, "made-by-" .. name)
                    local arguments = {
                        write = { scratch, "x" }, replace = { scratch, "x" }, copy = { existing, scratch },
                        remove = { scratch }, createDir = { scratch }, cwd = {}, list = { dir },
                    }
                    local ok, problem = pcall(jess.fs[name], table.unpack(arguments[name] or { existing }))
                    if allowed then
                        assert(ok, `{name} with {capability}: {problem}`)
                    else
                        local expected = `jess.fs.{name}: needs the '{capability}' capability, which this runtime does not grant`
                        assert(not ok and string.find(tostring(problem), expected, 1, true), `{name}: {problem}`)
                    end
                    "#,
                );
            }
        }
        run(
            &lua,
            r#"
            assert(jess.fs.join("a", "b") ~= nil and jess.fs.fileName("a/b.txt") == "b.txt")
            assert(jess.fs.parent("a/b.txt") == "a" and jess.fs.isFileName("b.txt"))
            "#,
        );
        if !may_write {
            let made = std::fs::read_dir(directory.path()).unwrap().count();
            assert_eq!(made, 1, "a runtime without filesystem.write changed the folder");
            assert_eq!(std::fs::read_to_string(&existing).unwrap(), "kept");
        }
        // What a granted run made, cleared for the next policy.
        for entry in std::fs::read_dir(directory.path()).unwrap().flatten() {
            if entry.path() != existing {
                if entry.path().is_dir() {
                    std::fs::remove_dir_all(entry.path()).unwrap();
                } else {
                    std::fs::remove_file(entry.path()).unwrap();
                }
            }
        }
        std::fs::write(&existing, "kept").unwrap();
    }
}

#[test]
fn files_read_write_and_list() {
    let directory = tempfile::tempdir().unwrap();
    let lua = common::filesystem_runtime();
    set_global(&lua, "dir", &directory.path().display().to_string());
    run(
        &lua,
        r#"
        local nested = jess.fs.join(dir, "a", "b")
        jess.fs.createDir(nested)
        assert(jess.fs.isDir(nested) and not jess.fs.isFile(nested))
        local file = jess.fs.join(nested, "Notes.TXT")
        assert(jess.fs.read(file) == nil and jess.fs.size(file) == nil and not jess.fs.exists(file))
        jess.fs.write(file, "hello")
        assert(jess.fs.read(file) == "hello" and jess.fs.size(file) == 5 and jess.fs.isFile(file))
        jess.fs.write(file, buffer.fromstring("bytes\0!"))
        assert(jess.fs.read(file) == "bytes\0!")
        local written = jess.fs.write(file, "again", { backup = true })
        assert(written.path == file and written.backup == file .. ".bak", tostring(written.backup))
        assert(jess.fs.read(written.backup) == "bytes\0!" and jess.fs.read(file) == "again")
        assert(jess.fs.write(file, "last").backup == nil, "no backup unless asked")
        jess.fs.remove(written.backup)
        local deeper = jess.fs.join(dir, "new", "folder", "made.txt")
        assert(jess.fs.write(deeper, "x").backup == nil and jess.fs.read(deeper) == "x")
        jess.fs.remove(deeper)
        assert(jess.fs.fileName(file) == "Notes.TXT" and jess.fs.parent(file) == nested)
        local copy = jess.fs.join(nested, "copy.txt")
        jess.fs.copy(file, copy)
        local names = jess.fs.list(nested)
        table.sort(names)
        assert(#names == 2 and names[1] == "Notes.TXT" and names[2] == "copy.txt")
        assert(jess.fs.remove(copy) and not jess.fs.remove(copy))
        assert(type(jess.fs.cwd()) == "string")
        "#,
    );
}

#[test]
fn toml_decodes_with_its_layout_through_the_one_codec() {
    run(
        &lua(),
        r#"
        local toml = require("@jess/data/toml")
        local value, layout = toml.decode([[
            zeta = 1
            alpha = 2.5
            flag = true
            list = [1, 2.0, "three"]

            [table.inner]
            name = "x"
            when = 1979-05-27
        ]])
        assert(value.zeta == 1 and value.alpha == 2.5 and value.flag == true)
        local keys = layout[value].keys
        assert(keys[1] == "zeta" and keys[2] == "alpha" and keys[3] == "flag" and keys[4] == "list")
        assert(layout[value].floats.alpha and not layout[value].floats.zeta)
        assert(layout[value.list].floats[2] and not layout[value.list].floats[1])
        assert(value.table.inner.name == "x" and value.table.inner.when == "1979-05-27")
        assert(layout[value.table.inner].datetimes.when and not layout[value.table.inner].datetimes.name)
        assert(not pcall(toml.decode, "key = "))
        assert(jess.toml == nil and not pcall(require, "@jess/toml"))
        "#,
    );
}

#[test]
fn regexes_match_like_rust() {
    run(
        &lua(),
        r#"
        local sensitive = jess.regex.new("^torch_\\d+$")
        assert(sensitive:isMatch("torch_01") and not sensitive:isMatch("Torch_01"))
        local insensitive = jess.regex.new("purple", { caseInsensitive = true })
        assert(insensitive:isMatch("A PURPLE light") and tostring(insensitive) == "purple")
        local ok, problem = pcall(jess.regex.new, "[")
        assert(not ok and tostring(problem):find("unclosed"), tostring(problem))
        "#,
    );
}

#[test]
fn regexes_find_capture_replace_split_and_iterate() {
    run(
        &lua(),
        r##"
        local number = jess.regex.new("(?P<whole>\\d+)(?:\\.(\\d+))?")
        assert(number.pattern == "(?P<whole>\\d+)(?:\\.(\\d+))?")

        local start, finish, text = number:find("light 12.5 and 7")
        assert(start == 7 and finish == 10 and text == "12.5")
        assert(number:find("no digits") == nil)
        local again = { number:find("light 12.5 and 7", 11) }
        assert(again[1] == 16 and again[2] == 16 and again[3] == "7")
        assert(number:find("light 12.5 and 7", -1) == 16)
        assert(number:match("x 42 y") == "42")

        local groups = number:captures("light 12.5")
        assert(groups[0] == "12.5" and groups[1] == "12" and groups[2] == "5" and groups.whole == "12")
        local partial = number:captures("light 7")
        assert(partial[1] == "7" and partial[2] == nil)
        assert(number:captures("none") == nil)

        local found = {}
        for match, first, last in number:matches("1 22 333") do
            table.insert(found, `{match}@{first}-{last}`)
        end
        assert(table.concat(found, ",") == "1@1-1,22@3-4,333@6-8", table.concat(found, ","))
        local empties = 0
        for _ in jess.regex.new("x*"):matches("ab") do empties += 1 end
        assert(empties == 3, `{empties}`)

        local once, count = number:replace("1 and 2", "<$whole>")
        assert(once == "<1> and 2" and count == 1)
        local all, total = number:replaceAll("1 and 2 and 3", "#")
        assert(all == "# and # and #" and total == 3)
        local limited = number:replaceAll("1 2 3", "#", 2)
        assert(limited == "# # 3")
        local doubled = number:replaceAll("1 and 20", function(groups)
            return tostring(tonumber(groups[1]) * 2)
        end)
        assert(doubled == "2 and 40")
        local kept, replaced = number:replaceAll("1 and 2", function(groups) return groups[1] == "1" and "one" or nil end)
        assert(kept == "one and 2" and replaced == 1, `{replaced}`)
        local skipped, two = number:replaceAll("1 2 3 4", function(groups) return groups[1] ~= "1" and "#" or nil end, 2)
        assert(skipped == "1 # # 4" and two == 2, skipped)
        local untouched, none = number:replace("1 and 2", function() return false end)
        assert(untouched == "1 and 2" and none == 0)

        local parts = jess.regex.new("\\s*,\\s*"):split("a , b,c")
        assert(#parts == 3 and parts[1] == "a" and parts[3] == "c")
        assert(#jess.regex.new(","):split("a,b,c", 2) == 2)

        assert(jess.regex.new(jess.regex.escape("a.b*")):isMatch("xa.b*") and not jess.regex.new(jess.regex.escape("a.b")):isMatch("axb"))
        assert(jess.regex.new("^b", { multiLine = true }):isMatch("a\nb"))
        assert(jess.regex.new("a.b", { dotAll = true }):isMatch("a\nb"))
        "##,
    );
}
