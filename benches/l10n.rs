//! Timed runs of the paths a GUI would hit: load, cached format, uncached format, locale switch,
//! hot reload. Plain `Instant` timing, best of several rounds, so the numbers read as
//! nanoseconds per call with nothing between them and the code.
//!
//! `cargo bench --bench l10n` (release). The corpus is synthetic: a bundle of 1000 messages in
//! four locales, 70% plain text, 15% one argument, 10% plural, 5% select with a number.

use std::hint::black_box;
use std::time::{Duration, Instant};

use jess::l10n::{BundleId, Locale, MemoryFiles, Registry, Value, VfsProvider};

const MESSAGES: usize = 1000;
const LOCALES: [&str; 4] = ["en", "pl", "de", "pt-BR"];

fn corpus(locale: &str, edit: usize) -> String {
    let mut file = String::new();
    for index in 0..MESSAGES {
        let line = match index % 20 {
            0 | 1 => format!(
                "message_{index} = '{{count, plural, =0 {{No things ({locale})}} one {{# thing}} few {{# things}} many {{# things}} other {{# things ({index})}}}}'\n"
            ),
            2 => format!(
                "message_{index} = '{{mode, select, edit {{Editing {{value, number, ::.00}}}} other {{Viewing {{value, number, :: group-off}} ({locale})}}}}'\n"
            ),
            3..=5 => format!("message_{index} = 'Hello, {{name}}! This is item {index} in {locale}.'\n"),
            _ => format!("message_{index} = 'A plain label, number {index}, in {locale}, edit {edit}.'\n"),
        };
        file.push_str(&line);
    }
    file
}

fn files(edit: usize) -> MemoryFiles {
    let files = MemoryFiles::new();
    for locale in LOCALES {
        files.insert(format!("l10n/bench/{locale}.toml"), corpus(locale, edit));
    }
    files
}

fn registry(files: &MemoryFiles) -> (Registry, BundleId) {
    let mut registry = Registry::new();
    registry.add_provider(VfsProvider::new("bench", "l10n", files.clone()));
    let report = registry.reload();
    assert!(report.is_clean(), "{:#?}", report.diagnostics);
    let bundle = registry.bundle("bench").unwrap();
    (registry, bundle)
}

/// Runs `f` in rounds of `iterations` and returns the best round's time per call.
fn time(mut f: impl FnMut(), iterations: u32, rounds: u32) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..rounds {
        let start = Instant::now();
        for _ in 0..iterations {
            f();
        }
        best = best.min(start.elapsed() / iterations);
    }
    best
}

fn report(name: &str, per_call: Duration, note: &str) {
    println!("{name:<40} {:>10.1} ns  {note}", per_call.as_secs_f64() * 1e9);
}

// One scenario after another; splitting it would only scatter the list.
#[allow(clippy::too_many_lines)]
fn main() {
    let files = files(0);
    let (mut registry, bundle) = registry(&files);
    let messages = u32::try_from(MESSAGES * LOCALES.len()).unwrap();

    let load = time(
        || {
            let mut fresh = Registry::new();
            fresh.add_provider(VfsProvider::new("bench", "l10n", files.clone()));
            black_box(fresh.reload().generation);
        },
        5,
        3,
    );
    report(
        "load + compile (4 locales x 1000)",
        load,
        &format!("{:.0} ns/message", (load / messages).as_secs_f64() * 1e9),
    );

    let plain = registry.message(bundle, "message_7");
    let one_arg = registry.message(bundle, "message_3");
    let plural = registry.message(bundle, "message_0");
    let select = registry.message(bundle, "message_2");
    let mut out = String::with_capacity(256);
    let name = [("name", Value::Str("Fargoth"))];
    let count = [("count", Value::Int(5))];
    let count_float = [("count", Value::Float(5.0))];
    let select_args = [("mode", Value::Str("edit")), ("value", Value::Float(1234.5))];

    let cached = |registry: &Registry, id, args: &[(&str, Value<'_>)], out: &mut String| {
        out.clear();
        registry.format(id, args, out).unwrap();
        black_box(out.len());
    };
    report(
        "literal (borrowed, no copy)",
        time(
            || {
                black_box(registry.literal(plain));
            },
            1_000_000,
            5,
        ),
        "",
    );
    report(
        "format cached: plain text",
        time(|| cached(&registry, plain, &[], &mut out), 1_000_000, 5),
        "",
    );
    report(
        "format cached: one string argument",
        time(|| cached(&registry, one_arg, &name, &mut out), 1_000_000, 5),
        "",
    );
    registry.set_preferred_locales(&[Locale::parse("pl").unwrap()]).unwrap();
    report(
        "format cached: plural (pl, integer)",
        time(|| cached(&registry, plural, &count, &mut out), 1_000_000, 5),
        "",
    );
    report(
        "format cached: plural (pl, float)",
        time(|| cached(&registry, plural, &count_float, &mut out), 1_000_000, 5),
        "",
    );
    report(
        "format cached: select + 2 numbers",
        time(|| cached(&registry, select, &select_args, &mut out), 500_000, 5),
        "",
    );
    report(
        "lookup by name + format (Luau path)",
        time(
            || {
                let id = registry.message_id(bundle, "message_3").unwrap();
                cached(&registry, id, &name, &mut out);
            },
            1_000_000,
            5,
        ),
        "",
    );

    let polish = registry.locale("pl").unwrap();
    let pattern = "{count, plural, =0 {No things} one {# thing} few {# things} many {# things} other {# things}}";
    report(
        "format uncached: compile + plural",
        time(
            || {
                out.clear();
                registry.format_pattern(polish, pattern, &count, &mut out).unwrap();
                black_box(out.len());
            },
            200_000,
            5,
        ),
        "",
    );

    let (german, polish) = (Locale::parse("de").unwrap(), Locale::parse("pl").unwrap());
    let mut flip = false;
    report(
        "locale switch (1 bundle, 1000 messages)",
        time(
            || {
                flip = !flip;
                let locale = if flip { &german } else { &polish };
                black_box(registry.set_preferred_locales(std::slice::from_ref(locale)).unwrap().get());
            },
            2_000,
            5,
        ),
        "",
    );

    let mut edit = 0;
    report(
        "hot reload after an edit (all 4 files)",
        time(
            || {
                edit += 1;
                files.insert("l10n/bench/en.toml", corpus("en", edit));
                assert!(black_box(registry.reload()).is_clean());
            },
            5,
            3,
        ),
        "",
    );
    files.insert("l10n/bench/de.toml", "broken = '{count, plural, one {#}'\n");
    report(
        "hot reload rejected (keeps last good)",
        time(
            || {
                let report = black_box(registry.reload());
                assert_eq!(report.kept, [bundle]);
            },
            5,
            3,
        ),
        "",
    );
}
