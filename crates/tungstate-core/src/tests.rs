//! The classifier, exercised without a filesystem.
//!
//! Every test here builds `Attributes` by hand, which is the whole point of
//! the crate having no I/O: a policy's behaviour is a function of data.

use std::collections::BTreeMap;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use proptest::prelude::*;

use crate::attrs::{Attributes, Tier, Value};
use crate::classify::{Outcome, RuleResult};
use crate::error::{PolicyError, Warning, line_of};
use crate::grammar::{Interval, parse_age, parse_duration, parse_size};
use crate::policy::{Loaded, Policy};
use crate::template::{Template, canonical_path, sanitize, slug};
use crate::vars::wildcard_matches;

/// 2024-06-01T12:00:00Z. Mid-year, mid-day, so no zone tips it over a
/// boundary by accident.
const NOON: Timestamp = Timestamp::constant(1_717_243_200, 0);

/// The policy from DESIGN.md §2, both skins: one big layout template and two
/// narrow rules.
const SKETCH: &str = r#"
[folder]
name = "downloads"
mode = "observe"
inbox = "_inbox"
ignore = [".DS_Store", "*.part", ".git/**"]
opaque = ["*.app", "**/node_modules"]
symlinks = "ignore"

[defaults]
on_duplicate = "trash"
on_conflict = "quarantine"
cooldown = "30s"

[[rule]]
name = "media-by-origin"
path = "{date:%Y}/{date:%m}/{source|default:\"unknown\"}/{category}/{ext|lower}/{size_range}"
match = { mime = ["image/*", "video/*"] }
vars.date       = { from = ["exif.DateTimeOriginal", "mtime"], tz = "utc" }
vars.category   = { from = "mime", map = { "image/*" = "Photos", "video/*" = "Videos" } }
vars.size_range = { from = "size", bucket = ["<10MiB", "10MiB-1GiB", ">1GiB"] }
rename = "{date:%Y%m%d_%H%M%S}_{hash:8}.{ext|lower}"

[[rule]]
name = "archives"
path = "Archives"
match = { ext = ["zip", "tar", "gz", "7z"], size = "> 1MiB" }

[[rule]]
name = "documents"
path = "Documents/{kind}"
match = { mime = ["application/pdf", "application/msword", "text/*"] }
vars.kind = { from = "mime", map = { "application/pdf" = "PDF", "text/*" = "Text", "_" = "Other" } }
"#;

fn load(text: &str) -> Loaded {
    match Policy::parse(text) {
        Ok(loaded) => loaded,
        Err(error) => panic!("policy should load: {}", diagnostic(text, &error)),
    }
}

fn policy(text: &str) -> Policy {
    load(text).policy
}

/// The shape a diagnostic takes in these tests: line and column, then the
/// message. The CLI draws the same span with `miette`; this proves the span
/// is right without depending on how it is drawn.
fn diagnostic(text: &str, error: &PolicyError) -> String {
    match error.span() {
        Some(span) => {
            let (line, column) = line_of(text, span.start);
            format!("line {line}, column {column}: {error}")
        }
        None => format!("no position: {error}"),
    }
}

fn file(path: &str, size: u64) -> Attributes {
    let mut attrs = Attributes::new(path, size, NOON);
    attrs.mtime = Some(NOON - jiff::SignedDuration::from_hours(48));
    attrs
}

fn photo(path: &str) -> Attributes {
    let mut attrs = file(path, 4 * 1024 * 1024);
    attrs.mime = Some("image/jpeg".to_string());
    attrs.exif.insert(
        "DateTimeOriginal".to_string(),
        "2023:12:25 08:30:00".to_string(),
    );
    attrs.hash = Some("0123456789abcdef".repeat(4));
    attrs
}

fn destination(policy: &Policy, attrs: &Attributes) -> String {
    match policy.explain(attrs).outcome {
        Outcome::Routed { destination, .. } => destination,
        other => panic!("expected a destination, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Grammars

#[test]
fn sizes_parse_in_binary_and_decimal_units() {
    assert_eq!(parse_size("4096").unwrap(), 4096);
    assert_eq!(parse_size("10MiB").unwrap(), 10 * 1024 * 1024);
    assert_eq!(parse_size("1.5 KiB").unwrap(), 1536);
    assert_eq!(parse_size("2gb").unwrap(), 2_000_000_000);
    assert_eq!(parse_size("1G").unwrap(), 1 << 30);
    assert!(parse_size("MiB").unwrap_err().contains("not a size"));
    assert!(
        parse_size("10 furlongs")
            .unwrap_err()
            .contains("not a size unit")
    );
}

#[test]
fn durations_parse_in_seconds_minutes_hours_days_weeks() {
    assert_eq!(parse_duration("30s").unwrap().as_secs(), 30);
    assert_eq!(parse_duration("5m").unwrap().as_secs(), 300);
    assert_eq!(parse_duration("2h").unwrap().as_secs(), 7200);
    assert_eq!(parse_age("30d").unwrap(), 30 * 86_400);
    assert_eq!(parse_age("1w").unwrap(), 7 * 86_400);
    assert!(
        parse_duration("soon")
            .unwrap_err()
            .contains("not a duration")
    );
    assert!(
        parse_duration("3 fortnights")
            .unwrap_err()
            .contains("not a time unit")
    );
}

#[test]
fn intervals_read_comparisons_and_ranges() {
    let over = Interval::parse("> 1MiB", parse_size).unwrap();
    assert!(!over.contains(1 << 20));
    assert!(over.contains((1 << 20) + 1));

    let at_least = Interval::parse(">= 1MiB", parse_size).unwrap();
    assert!(at_least.contains(1 << 20));

    let under = Interval::parse("< 10MiB", parse_size).unwrap();
    assert!(under.contains((10 << 20) - 1));
    assert!(!under.contains(10 << 20));

    // Both spellings of a range, both ends inclusive.
    for text in ["1MiB..1GiB", "1MiB-1GiB"] {
        let range = Interval::parse(text, parse_size).unwrap();
        assert!(range.contains(1 << 20), "{text} should include its low end");
        assert!(
            range.contains(1 << 30),
            "{text} should include its high end"
        );
        assert!(!range.contains((1 << 20) - 1));
        assert!(!range.contains((1 << 30) + 1));
    }

    assert!(
        Interval::parse("1GiB..1MiB", parse_size)
            .unwrap_err()
            .contains("backwards")
    );
    assert!(
        Interval::parse("about 1MiB", parse_size)
            .unwrap_err()
            .contains("not a comparison")
    );
    assert!(
        Interval::parse("", parse_size)
            .unwrap_err()
            .contains("empty")
    );
}

#[test]
fn bucket_labels_are_legal_directory_names() {
    assert_eq!(Interval::label("<10MiB"), "under-10MiB");
    assert_eq!(Interval::label("10MiB-1GiB"), "10MiB-1GiB");
    assert_eq!(Interval::label("10MiB..1GiB"), "10MiB-1GiB");
    assert_eq!(Interval::label("> 1GiB"), "over-1GiB");
    assert_eq!(Interval::label(">= 1GiB"), "from-1GiB");
    assert_eq!(Interval::label("<= 1GiB"), "up-to-1GiB");
    assert_eq!(Interval::label("= 4096"), "exactly-4096");
    for label in ["<10MiB", "10MiB-1GiB", ">1GiB"] {
        assert_eq!(sanitize(&Interval::label(label)), Interval::label(label));
    }
}

#[test]
fn interval_subsumption_is_exact_at_the_edges() {
    let wide = Interval::parse("1MiB..1GiB", parse_size).unwrap();
    let narrow = Interval::parse("2MiB..500MiB", parse_size).unwrap();
    let open_low = Interval::parse("< 1GiB", parse_size).unwrap();
    let strict = Interval::parse("> 1MiB", parse_size).unwrap();
    let lenient = Interval::parse(">= 1MiB", parse_size).unwrap();

    assert!(wide.subsumes(&narrow));
    assert!(!narrow.subsumes(&wide));
    assert!(open_low.subsumes(&narrow));
    assert!(!wide.subsumes(&open_low));
    // `>= 1MiB` takes 1MiB itself; `> 1MiB` does not, so it cannot cover it.
    assert!(lenient.subsumes(&strict));
    assert!(!strict.subsumes(&lenient));
}

// ---------------------------------------------------------------------------
// Templates and filters

fn render(template: &str, values: &[(&str, Value)]) -> String {
    let template = Template::parse(template).expect("template should parse");
    let map: BTreeMap<&str, Value> = values.iter().cloned().collect();
    template
        .render(|name| map.get(name).cloned().map(|v| (v, TimeZone::UTC)))
        .expect("template should render")
        .text
}

#[test]
fn every_filter_does_what_its_name_says() {
    let name = || Value::Text("Holiday Snaps 2024!.JPG".to_string());
    assert_eq!(
        render("{n|lower}", &[("n", name())]),
        "holiday snaps 2024!.jpg"
    );
    assert_eq!(
        render("{n|upper}", &[("n", name())]),
        "HOLIDAY SNAPS 2024!.JPG"
    );
    assert_eq!(
        render("{n|slug}", &[("n", name())]),
        "holiday-snaps-2024-jpg"
    );
    assert_eq!(render("{n|trunc:7}", &[("n", name())]), "Holiday");
    assert_eq!(
        render("{n|pad:3}", &[("n", Value::Text("7".into()))]),
        "007"
    );
    assert_eq!(
        render("{n|pad:1}", &[("n", Value::Text("42".into()))]),
        "42"
    );
    assert_eq!(
        render("{n|hash:8}", &[("n", Value::Text("hello".into()))]),
        &blake3::hash(b"hello").to_hex()[..8]
    );
    assert_eq!(
        render("{n|sanitize}", &[("n", Value::Text("a:b/c?d".into()))]),
        "a_b_c_d"
    );
    assert_eq!(render("{n|default:\"none\"}", &[]), "none");
    assert_eq!(
        render("{n|default:\"none\"}", &[("n", Value::Text("set".into()))]),
        "set"
    );
}

#[test]
fn filters_chain_left_to_right() {
    let value = Value::Text("Report FINAL v2.PDF".to_string());
    assert_eq!(
        render("{n|slug|trunc:12|upper}", &[("n", value.clone())]),
        "REPORT-FINAL"
    );
    // A default that is then filtered: the default is the input to the chain.
    assert_eq!(render("{n|default:\"No Name\"|slug}", &[]), "no-name");
    // A size renders as bytes; text formats need a length.
    assert_eq!(render("{s}", &[("s", Value::Size(1536))]), "1536");
    assert_eq!(
        render("{h:8}", &[("h", Value::Text("0123456789abcdef".into()))]),
        "01234567"
    );
}

#[test]
fn dates_format_with_strftime_in_the_given_zone() {
    let template = Template::parse("{t:%Y-%m-%d %H:%M}").unwrap();
    let show = |tz: TimeZone| {
        template
            .render(|_| Some((Value::Instant(NOON), tz.clone())))
            .unwrap()
            .text
    };
    assert_eq!(show(TimeZone::UTC), "2024-06-01 12:00");
    assert_eq!(
        show(TimeZone::get("Africa/Lagos").unwrap()),
        "2024-06-01 13:00"
    );
    let local = NOON
        .to_zoned(TimeZone::system())
        .strftime("%Y-%m-%d %H:%M")
        .to_string();
    assert_eq!(show(TimeZone::system()), local);

    // A camera time is a wall-clock reading and is never shifted by a zone.
    let civil = jiff::civil::date(2023, 12, 25).at(8, 30, 0, 0);
    let shown = template
        .render(|_| Some((Value::Civil(civil), TimeZone::get("Africa/Lagos").unwrap())))
        .unwrap()
        .text;
    assert_eq!(shown, "2023-12-25 08:30");
    // No format: the default is a date.
    assert_eq!(render("{t}", &[("t", Value::Instant(NOON))]), "2024-06-01");
}

#[test]
fn braces_can_be_written_literally() {
    assert_eq!(
        render("{{literal}} {n}", &[("n", Value::Text("x".into()))]),
        "{literal} x"
    );
}

#[test]
fn template_parse_errors_point_at_the_character() {
    let error = Template::parse("a/{date").unwrap_err();
    assert_eq!(error.span, 2..3);
    assert!(error.message.contains("never closed"));

    let error = Template::parse("a}b").unwrap_err();
    assert_eq!(error.span, 1..2);

    let error = Template::parse("{n|frobnicate}").unwrap_err();
    assert!(error.message.contains("not a filter"), "{}", error.message);
    assert_eq!(&"{n|frobnicate}"[error.span], "frobnicate");

    let error = Template::parse("{n|trunc}").unwrap_err();
    assert!(error.message.contains("needs a number"));

    let error = Template::parse("{n|default:x}").unwrap_err();
    assert!(error.message.contains("quoted"));

    let error = Template::parse("{}").unwrap_err();
    assert!(error.message.contains("needs a name"));
}

#[test]
fn sanitize_strips_what_no_backend_accepts() {
    assert_eq!(sanitize("a<b>c:d\"e/f\\g|h?i*j"), "a_b_c_d_e_f_g_h_i_j");
    assert_eq!(sanitize("trailing. . "), "trailing");
    assert_eq!(sanitize("tab\there"), "tab_here");
    assert_eq!(sanitize("CON"), "_CON");
    assert_eq!(sanitize("con.txt"), "_con.txt");
    assert_eq!(sanitize("console.log"), "console.log");
    assert_eq!(sanitize("plain-name_1.txt"), "plain-name_1.txt");
    assert_eq!(slug("  Ünïcode -- Snaps! "), "n-code-snaps");
}

#[test]
fn canonical_path_never_escapes() {
    assert_eq!(canonical_path("a/b/c.txt"), "a/b/c.txt");
    assert_eq!(canonical_path("/a//b/"), "a/b");
    assert_eq!(canonical_path("../../etc/passwd"), "etc/passwd");
    assert_eq!(canonical_path("a/./b/../c"), "a/b/c");
    assert_eq!(canonical_path("a\\b"), "a/b");
    assert_eq!(canonical_path("."), "");
    // Trailing dots are stripped by the Windows rule, so `...` empties out.
    assert_eq!(canonical_path("a/..."), "a");
}

#[test]
fn sanitize_runs_last_on_every_segment_whether_or_not_it_is_written() {
    let policy = policy(
        r#"
[folder]
name = "x"
[[rule]]
name = "raw"
path = "by-name/{name|upper}"
rename = "{stem}.{ext}"
"#,
    );
    let attrs = file("odd:name?.TXT", 1);
    assert_eq!(
        destination(&policy, &attrs),
        "by-name/ODD_NAME_.TXT/odd_name_.TXT"
    );
}

// ---------------------------------------------------------------------------
// Vars: fallback chains, map, bucket

#[test]
fn map_prefers_exact_then_most_specific_wildcard_then_catch_all() {
    let policy = policy(
        r#"
[folder]
name = "x"
[[rule]]
name = "kinds"
path = "{kind}"
vars.kind = { from = "mime", map = { "image/png" = "PNG", "image/*" = "Image", "*" = "Anything", "_" = "Other" } }
"#,
    );
    let with = |mime: &str| {
        let mut attrs = file("f.bin", 1);
        attrs.mime = Some(mime.to_string());
        destination(&policy, &attrs)
    };
    assert_eq!(with("image/png"), "PNG/f.bin");
    assert_eq!(with("image/jpeg"), "Image/f.bin");
    assert_eq!(with("video/mp4"), "Anything/f.bin");
}

#[test]
fn map_without_a_catch_all_leaves_the_rule_unresolvable() {
    let policy = policy(
        r#"
[folder]
name = "x"
[[rule]]
name = "kinds"
path = "{kind}"
vars.kind = { from = "mime", map = { "image/*" = "Image" } }
"#,
    );
    let mut attrs = file("f.bin", 1);
    attrs.mime = Some("video/mp4".to_string());
    match policy.explain(&attrs).outcome {
        Outcome::Unresolvable { var, reason, .. } => {
            assert_eq!(var, "kind");
            assert!(reason.contains("no value"), "{reason}");
        }
        other => panic!("expected unresolvable, got {other:?}"),
    }
    // `_` fixes it without touching the other entries.
    let policy = self::policy(
        r#"
[folder]
name = "x"
[[rule]]
name = "kinds"
path = "{kind}"
vars.kind = { from = "mime", map = { "image/*" = "Image", "_" = "Other" } }
"#,
    );
    assert_eq!(destination(&policy, &attrs), "Other/f.bin");
}

#[test]
fn wildcards_match_anywhere_in_the_text() {
    assert!(wildcard_matches("image/*", "image/png"));
    assert!(wildcard_matches("*", ""));
    assert!(wildcard_matches("*.tar.*", "a.tar.gz"));
    assert!(!wildcard_matches("image/*", "video/mp4"));
    assert!(wildcard_matches("exact", "EXACT"));
    assert!(!wildcard_matches("exact", "exactly"));
}

#[test]
fn every_bucket_boundary_lands_on_the_right_side() {
    let policy = policy(SKETCH);
    let mib = 1_u64 << 20;
    let gib = 1_u64 << 30;
    let range_for = |size: u64| {
        let mut attrs = photo("a.jpg");
        attrs.size = size;
        let dest = destination(&policy, &attrs);
        dest.rsplit('/').nth(1).unwrap().to_string()
    };
    assert_eq!(range_for(0), "under-10MiB");
    assert_eq!(range_for(10 * mib - 1), "under-10MiB");
    assert_eq!(range_for(10 * mib), "10MiB-1GiB");
    assert_eq!(range_for(gib), "10MiB-1GiB");
    assert_eq!(range_for(gib + 1), "over-1GiB");
    assert_eq!(range_for(u64::MAX), "over-1GiB");
}

#[test]
fn a_gap_between_buckets_is_reported_not_guessed() {
    let policy = policy(
        r#"
[folder]
name = "x"
[[rule]]
name = "sized"
path = "{range}"
vars.range = { from = "size", bucket = ["<1MiB", ">10MiB"] }
"#,
    );
    let attrs = file("f", 5 << 20);
    match policy.explain(&attrs).outcome {
        Outcome::Unresolvable { vars, .. } => {
            let missing = vars[0].missing.as_ref().unwrap().to_string();
            assert!(missing.contains("falls in no bucket"), "{missing}");
        }
        other => panic!("expected unresolvable, got {other:?}"),
    }
}

#[test]
fn a_from_chain_falls_through_absent_sources() {
    let policy = policy(SKETCH);

    let with_exif = photo("IMG_0001.JPG");
    let trace = policy.explain(&with_exif);
    let Outcome::Routed {
        vars, destination, ..
    } = &trace.outcome
    else {
        panic!("{trace:?}");
    };
    let date = vars.iter().find(|v| v.name == "date").unwrap();
    assert_eq!(date.source.as_deref(), Some("exif.DateTimeOriginal"));
    assert_eq!(date.tier, Some(Tier::Meta));
    assert!(date.skipped.is_empty());
    assert!(destination.starts_with("2023/12/"), "{destination}");

    let mut without = with_exif.clone();
    without.exif.clear();
    let trace = policy.explain(&without);
    let Outcome::Routed {
        vars, destination, ..
    } = &trace.outcome
    else {
        panic!("{trace:?}");
    };
    let date = vars.iter().find(|v| v.name == "date").unwrap();
    assert_eq!(date.source.as_deref(), Some("mtime"));
    assert_eq!(date.tier, Some(Tier::Stat));
    assert_eq!(date.skipped, vec!["exif.DateTimeOriginal".to_string()]);
    assert!(destination.starts_with("2024/05/"), "{destination}");
}

#[test]
fn the_sketch_routes_a_photo_end_to_end() {
    let policy = policy(SKETCH);
    let photo = photo("IMG_0001.JPG");
    assert_eq!(
        destination(&policy, &photo),
        "2023/12/unknown/Photos/jpg/under-10MiB/20231225_083000_01234567.jpg"
    );
    // `source` from the journal fills the `default:"unknown"` slot.
    let mut from_link = photo;
    from_link.source = Some("phone".to_string());
    assert_eq!(
        destination(&policy, &from_link),
        "2023/12/phone/Photos/jpg/under-10MiB/20231225_083000_01234567.jpg"
    );
}

#[test]
fn regex_captures_become_variables() {
    let policy = policy(
        r#"
[folder]
name = "tv"
[[rule]]
name = "episodes"
path = "Shows/{show|slug}/Season {season|pad:2}"
match = { regex = '^(?P<show>.+?)[. ]S(?P<season>\d+)E\d+' }
"#,
    );
    let attrs = file("Some.Show.S3E07.1080p.mkv", 1);
    assert_eq!(
        destination(&policy, &attrs),
        "Shows/some-show/Season 03/Some.Show.S3E07.1080p.mkv"
    );
}

// ---------------------------------------------------------------------------
// Precedence

#[test]
fn the_first_matching_rule_in_file_order_wins() {
    let policy = policy(SKETCH);
    let mut pdf = file("paper.pdf", 2 << 20);
    pdf.mime = Some("application/pdf".to_string());
    let trace = policy.explain(&pdf);
    let results: Vec<(&str, &RuleResult)> = trace
        .rules
        .iter()
        .map(|r| (r.name.as_str(), &r.result))
        .collect();
    assert!(
        matches!(results[0], ("media-by-origin", RuleResult::Failed { constraint, .. }) if constraint == "mime")
    );
    assert!(
        matches!(results[1], ("archives", RuleResult::Failed { constraint, .. }) if constraint == "ext")
    );
    assert!(matches!(results[2], ("documents", RuleResult::Matched)));
    assert_eq!(destination(&policy, &pdf), "Documents/PDF/paper.pdf");
}

#[test]
fn reordering_two_rules_changes_the_answer_and_reverting_reverts_it() {
    const A: &str =
        "[[rule]]\nname = \"pictures\"\npath = \"Pictures\"\nmatch = { mime = \"image/*\" }\n";
    const B: &str = "[[rule]]\nname = \"big\"\npath = \"Big\"\nmatch = { size = \"> 1MiB\" }\n";
    let head = "[folder]\nname = \"x\"\n";

    let mut big_picture = file("wall.png", 5 << 20);
    big_picture.mime = Some("image/png".to_string());

    let ab = policy(&format!("{head}{A}{B}"));
    let ba = policy(&format!("{head}{B}{A}"));
    assert_eq!(destination(&ab, &big_picture), "Pictures/wall.png");
    assert_eq!(destination(&ba, &big_picture), "Big/wall.png");

    // The loser is still evaluated, and the trace says it would have taken
    // the file: that is what tells a reader reordering is the lever.
    let trace = ab.explain(&big_picture);
    assert!(matches!(
        &trace.rules[1].result,
        RuleResult::AlsoMatches { taken_by } if taken_by == "pictures"
    ));

    // And back again: reverting the reorder reverts the structure.
    let ab_again = policy(&format!("{head}{A}{B}"));
    assert_eq!(destination(&ab_again, &big_picture), "Pictures/wall.png");
}

#[test]
fn a_rule_with_no_match_takes_everything_left() {
    let policy = policy(
        r#"
[folder]
name = "x"
[[rule]]
name = "zips"
path = "Zips"
match = { ext = "zip" }
[[rule]]
name = "everything-else"
path = "Misc/{ext|lower|default:\"none\"}"
"#,
    );
    assert_eq!(destination(&policy, &file("a.zip", 1)), "Zips/a.zip");
    assert_eq!(destination(&policy, &file("a.TXT", 1)), "Misc/txt/a.TXT");
    assert_eq!(destination(&policy, &file("README", 1)), "Misc/README");
}

#[test]
fn unmatched_files_go_to_the_inbox_when_there_is_one() {
    let policy = policy(SKETCH);
    let attrs = file("mystery.bin", 1);
    assert_eq!(
        policy.explain(&attrs).outcome,
        Outcome::Unmatched {
            inbox: Some("_inbox".to_string())
        }
    );
}

#[test]
fn ignore_symlinks_and_opaque_come_before_any_rule() {
    let policy = policy(SKETCH);
    let ignored = file("sub/.DS_Store", 1);
    assert!(
        matches!(policy.explain(&ignored).outcome, Outcome::Ignored { because } if because.contains(".DS_Store"))
    );
    let partial = file("movie.mp4.part", 1);
    assert!(
        matches!(policy.explain(&partial).outcome, Outcome::Ignored { because } if because.contains("*.part"))
    );
    let in_git = file(".git/objects/ab/cdef", 1);
    assert!(
        matches!(policy.explain(&in_git).outcome, Outcome::Ignored { because } if because.contains(".git/**"))
    );

    let mut link = photo("link.jpg");
    link.is_symlink = true;
    assert!(
        matches!(policy.explain(&link).outcome, Outcome::Ignored { because } if because.contains("symlinks"))
    );

    let inside_app = photo("Foo.app/Contents/Resources/icon.jpg");
    assert!(
        matches!(policy.explain(&inside_app).outcome, Outcome::Opaque { pattern } if pattern == "*.app")
    );
    let node = file("proj/node_modules/x/index.js", 1);
    assert!(
        matches!(policy.explain(&node).outcome, Outcome::Opaque { pattern } if pattern == "**/node_modules")
    );
}

#[test]
fn a_file_already_in_place_says_so() {
    let policy = policy(SKETCH);
    let mut zip = file("Archives/big.zip", 5 << 20);
    zip.mime = Some("application/zip".to_string());
    match policy.explain(&zip).outcome {
        Outcome::Routed {
            destination,
            in_place,
            ..
        } => {
            assert_eq!(destination, "Archives/big.zip");
            assert!(in_place);
        }
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Tiers

#[test]
fn required_tier_is_the_most_any_rule_needs() {
    assert_eq!(policy(SKETCH).required_tier(), Tier::Whole);

    let head = "[folder]\nname = \"x\"\n";
    let stat_only = policy(&format!(
        "{head}[[rule]]\nname = \"a\"\npath = \"A/{{ext}}\"\nmatch = {{ ext = \"zip\", size = \"> 1MiB\" }}\n"
    ));
    assert_eq!(stat_only.required_tier(), Tier::Stat);

    let sniffs = policy(&format!(
        "{head}[[rule]]\nname = \"a\"\npath = \"A\"\nmatch = {{ mime = \"image/*\" }}\n"
    ));
    assert_eq!(sniffs.required_tier(), Tier::Head);

    let exif = policy(&format!(
        "{head}[[rule]]\nname = \"a\"\npath = \"{{d:%Y}}\"\nvars.d = {{ from = [\"exif.DateTimeOriginal\", \"mtime\"] }}\n"
    ));
    assert_eq!(exif.required_tier(), Tier::Meta);

    let hashes = policy(&format!(
        "{head}[[rule]]\nname = \"a\"\npath = \"A\"\nrename = \"{{hash:8}}.{{ext}}\"\n"
    ));
    assert_eq!(hashes.required_tier(), Tier::Whole);

    // `|hash:N` hashes the value, not the file, so it costs nothing.
    let filter = policy(&format!(
        "{head}[[rule]]\nname = \"a\"\npath = \"A\"\nrename = \"{{name|hash:8}}\"\n"
    ));
    assert_eq!(filter.required_tier(), Tier::Stat);
    assert!(Tier::Stat < Tier::Head && Tier::Head < Tier::Meta && Tier::Meta < Tier::Whole);
}

// ---------------------------------------------------------------------------
// Shadowing

#[test]
fn a_literal_overlap_warns_and_names_both_rules() {
    let text = r#"
[folder]
name = "x"
[[rule]]
name = "images"
path = "Images"
match = { mime = "image/*" }
[[rule]]
name = "pngs"
path = "PNG"
match = { mime = "image/png" }
[[rule]]
name = "small-zips"
path = "Zips"
match = { ext = "zip", size = "< 1MiB" }
[[rule]]
name = "zips"
path = "Zips"
match = { ext = ["zip", "7z"] }
"#;
    let loaded = load(text);
    assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
    let Warning::Shadowed {
        rule,
        by,
        span,
        by_span,
    } = &loaded.warnings[0];
    assert_eq!((rule.as_str(), by.as_str()), ("pngs", "images"));
    assert_eq!(line_of(text, span.start).0, 9);
    assert_eq!(line_of(text, by_span.start).0, 5);
    assert_eq!(
        loaded.warnings[0].to_string(),
        "rule `pngs` can never fire: everything it matches is taken by `images` first"
    );
}

#[test]
fn a_catch_all_shadows_everything_after_it() {
    let loaded = load(
        r#"
[folder]
name = "x"
[[rule]]
name = "everything"
path = "All"
[[rule]]
name = "zips"
path = "Zips"
match = { ext = "zip" }
"#,
    );
    assert!(
        matches!(&loaded.warnings[..], [Warning::Shadowed { rule, by, .. }] if rule == "zips" && by == "everything")
    );
}

#[test]
fn shadowing_stays_silent_when_a_glob_or_regex_is_involved() {
    let loaded = load(
        r#"
[folder]
name = "x"
[[rule]]
name = "any-image"
path = "Images"
match = { glob = "*.png" }
[[rule]]
name = "pngs"
path = "PNG"
match = { glob = "*.png" }
[[rule]]
name = "named"
path = "Named"
match = { regex = "^IMG_" }
[[rule]]
name = "named-too"
path = "Named2"
match = { regex = "^IMG_" }
"#,
    );
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
}

// ---------------------------------------------------------------------------
// Diagnostics

fn failure(text: &str) -> String {
    match Policy::parse(text) {
        Ok(_) => panic!("policy should not load"),
        Err(error) => diagnostic(text, &error),
    }
}

#[test]
fn a_bad_mode_is_reported_on_its_line() {
    let text = "[folder]\nname = \"x\"\nmode = \"aggressive\"\n";
    insta::assert_snapshot!(failure(text));
}

#[test]
fn an_unknown_filter_is_reported_on_its_line() {
    let text = "[folder]\nname = \"x\"\n\n[[rule]]\nname = \"a\"\npath = \"A/{name|shout}\"\n";
    insta::assert_snapshot!(failure(text));
}

#[test]
fn an_unterminated_placeholder_is_reported_on_its_line() {
    let text = "[folder]\nname = \"x\"\n\n[[rule]]\nname = \"a\"\npath = \"A\"\nrename = \"{date:%Y.{ext}\"\n";
    insta::assert_snapshot!(failure(text));
}

#[test]
fn an_unknown_variable_is_reported_on_its_line() {
    let text = "[folder]\nname = \"x\"\n\n[[rule]]\nname = \"a\"\npath = \"{year}/{ext}\"\nvars.date = { from = \"mtime\" }\n";
    insta::assert_snapshot!(failure(text));
}

#[test]
fn the_other_loader_errors_point_somewhere_useful() {
    let head = "[folder]\nname = \"x\"\n\n";
    let cases = [
        format!("{head}[[rule]]\nname = \"a\"\npath = \"A\"\nmatch = {{ size = \"big\" }}\n"),
        format!("{head}[[rule]]\nname = \"a\"\npath = \"A\"\nmatch = {{ mime = \"jpeg\" }}\n"),
        format!("{head}[[rule]]\nname = \"a\"\npath = \"A\"\nmatch = {{ regex = \"(\" }}\n"),
        format!(
            "{head}[[rule]]\nname = \"a\"\npath = \"{{d}}\"\nvars.d = {{ from = \"mtime\", tz = \"Mars/Olympus\" }}\n"
        ),
        format!(
            "{head}[[rule]]\nname = \"a\"\npath = \"{{d}}\"\nvars.d = {{ from = \"colour\" }}\n"
        ),
        format!(
            "{head}[[rule]]\nname = \"a\"\npath = \"{{d}}\"\nvars.d = {{ from = \"name\", bucket = [\"<1MiB\"] }}\n"
        ),
        format!("{head}[[rule]]\nname = \"a\"\npath = \"{{mtime:8}}\"\n"),
        format!("{head}[[rule]]\nname = \"a\"\npath = \"{{name:%Y}}\"\n"),
        format!("{head}[[rule]]\nname = \"a\"\npath = \"{{mtime:%Q}}\"\n"),
        format!("{head}[[rule]]\nname = \"a\"\npath = \"{{name|sanitize|lower}}\"\n"),
        format!(
            "{head}[[rule]]\nname = \"a\"\npath = \"A\"\n[[rule]]\nname = \"a\"\npath = \"B\"\n"
        ),
        format!("{head}[[rule]]\nname = \"a\"\npath = \"A\"\npriority = 3\n"),
        format!("{head}[defaults]\ncooldown = \"a while\"\n"),
        format!("{head}[[rule]]\npath = \"A\"\n"),
    ];
    let rendered: Vec<String> = cases.iter().map(|text| failure(text)).collect();
    insta::assert_snapshot!(rendered.join("\n"));
}

#[test]
fn a_duplicate_rule_points_at_both_declarations() {
    let text = "[folder]\nname = \"x\"\n[[rule]]\nname = \"a\"\npath = \"A\"\n[[rule]]\nname = \"a\"\npath = \"B\"\n";
    let error = Policy::parse(text).unwrap_err();
    let (label, first) = error.related().unwrap();
    assert_eq!(label, "first declared here");
    assert_eq!(line_of(text, first.start).0, 4);
    assert_eq!(line_of(text, error.span().unwrap().start).0, 7);
}

#[test]
fn line_of_counts_from_one() {
    assert_eq!(line_of("ab\ncd", 0), (1, 1));
    assert_eq!(line_of("ab\ncd", 3), (2, 1));
    assert_eq!(line_of("ab\ncd", 4), (2, 2));
    assert_eq!(line_of("ab", 99), (1, 3));
}

#[test]
fn the_json_shape_of_a_trace_is_stable() {
    let policy = policy(SKETCH);
    let trace = policy.explain(&photo("IMG_0001.JPG"));
    let json = serde_json::to_value(&trace).unwrap();
    assert_eq!(json["tier"], "whole");
    assert_eq!(json["outcome"]["kind"], "routed");
    assert_eq!(json["outcome"]["rule"], "media-by-origin");
    assert_eq!(json["rules"][1]["result"]["kind"], "failed");
    assert_eq!(json["rules"][1]["result"]["constraint"], "ext");
    let date = &json["outcome"]["vars"][0];
    assert_eq!(date["name"], "date");
    assert_eq!(date["source"], "exif.DateTimeOriginal");
    assert_eq!(date["tier"], "meta");
}

// ---------------------------------------------------------------------------
// Properties

/// A placeholder a generated template may use, with its filters.
fn placeholder() -> impl Strategy<Value = String> {
    let names = prop::sample::select(vec![
        "name",
        "stem",
        "ext",
        "parent",
        "path",
        "size",
        "mtime",
        "mime",
        "hash",
        "exif.Make",
        "kind",
        "range",
        "show",
    ]);
    let filters = prop::collection::vec(
        prop::sample::select(vec![
            "lower",
            "upper",
            "slug",
            "trunc:3",
            "pad:5",
            "hash:6",
            "default:\"dflt\"",
        ]),
        0..3,
    );
    (names, filters, any::<bool>()).prop_map(|(name, filters, sanitize)| {
        let mut out = format!("{{{name}");
        if name == "mtime" {
            out.push_str(":%Y/%m");
        }
        for f in filters {
            out.push('|');
            out.push_str(f);
        }
        if sanitize {
            out.push_str("|sanitize");
        }
        out.push('}');
        out
    })
}

/// A template: literal runs (which may contain `..`, `/` and `\`) and
/// placeholders, interleaved.
fn template() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            "[a-zA-Z0-9 ._/-]{0,6}".prop_map(String::from),
            Just("..".to_string()),
            Just("/../".to_string()),
            Just("\\\\".to_string()),
            placeholder(),
        ],
        1..6,
    )
    .prop_map(|parts| parts.concat())
}

fn attributes() -> impl Strategy<Value = Attributes> {
    (
        "[^/\\x00]{1,24}",
        prop::collection::vec("[^/\\x00]{1,8}", 0..3),
        any::<u64>(),
        prop::option::of("[a-z]+/[a-z.+-]+"),
        any::<bool>(),
    )
        .prop_map(|(name, parents, size, mime, has_exif)| {
            let path = parents
                .iter()
                .map(String::as_str)
                .chain(std::iter::once(name.as_str()))
                .collect::<Vec<_>>()
                .join("/");
            let mut attrs = Attributes::new(&path, size, NOON);
            attrs.mtime = Some(NOON);
            attrs.mime = mime;
            attrs.hash = Some("f".repeat(64));
            if has_exif {
                attrs
                    .exif
                    .insert("Make".to_string(), "../Evil\\Corp".to_string());
            }
            attrs
        })
}

fn generated_policy(path: &str, rename: &str) -> Policy {
    let text = format!(
        "[folder]\nname = \"p\"\n[[rule]]\nname = \"only\"\npath = \"{path}\"\nrename = \"{rename}\"\n\
         match = {{ regex = \"^(?P<show>.{{0,3}})\" }}\n\
         vars.kind = {{ from = \"mime\", map = {{ \"image/*\" = \"Photos\", \"_\" = \"Other\" }} }}\n\
         vars.range = {{ from = \"size\", bucket = [\"<10MiB\", \"10MiB-1GiB\", \">1GiB\"] }}\n",
        path = path.replace('\\', "\\\\").replace('"', "\\\""),
        rename = rename.replace('\\', "\\\\").replace('"', "\\\""),
    );
    self::policy(&text)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn classification_is_deterministic(path in template(), rename in template(), attrs in attributes()) {
        let policy = generated_policy(&path, &rename);
        prop_assert_eq!(policy.explain(&attrs), policy.explain(&attrs));
    }

    #[test]
    fn the_traceless_decision_is_the_same_decision(path in template(), rename in template(), attrs in attributes()) {
        // `place` exists only to skip building a trace nobody reads when
        // classifying a whole folder. The moment it answers differently from
        // `explain`, what the command line showed you stops being what the
        // plan does — so the two are held together here rather than by care.
        let policy = generated_policy(&path, &rename);
        prop_assert_eq!(policy.place(&attrs), policy.explain(&attrs).outcome);
    }

    #[test]
    fn a_destination_is_always_relative_and_never_escapes(path in template(), rename in template(), attrs in attributes()) {
        let policy = generated_policy(&path, &rename);
        let trace = policy.explain(&attrs);
        if let Outcome::Routed { destination, .. } = &trace.outcome {
            prop_assert!(!destination.is_empty());
            prop_assert!(!destination.starts_with('/'));
            for component in std::path::Path::new(destination).components() {
                prop_assert!(
                    matches!(component, std::path::Component::Normal(_)),
                    "`{destination}` has a non-normal component {component:?}"
                );
            }
            for segment in destination.split('/') {
                prop_assert!(segment != "." && segment != ".." && !segment.is_empty());
                prop_assert!(!segment.contains(['\\', ':', '*', '?', '"', '<', '>', '|']), "`{segment}`");
                prop_assert!(!segment.ends_with('.') && !segment.ends_with(' '), "`{segment}`");
            }
        }
    }
}

#[test]
fn relocating_keeps_everything_that_was_read_off_the_file() {
    // The property the paper-apply depends on: a file that moves is the same
    // file, so its mime, its EXIF and its hash follow it unchanged and only
    // its name and parent move.
    let mut attrs = Attributes::new("old/place/clip.mp4", 42, jiff::Timestamp::UNIX_EPOCH);
    attrs.mime = Some("video/mp4".to_string());
    attrs.hash = Some("abc123".to_string());
    let before = attrs.clone();

    attrs.relocate("2026/06/clip.mp4");

    assert_eq!(attrs.parent, "2026/06");
    assert_eq!(attrs.name, "clip.mp4");
    assert_eq!(attrs.relative_path(), "2026/06/clip.mp4");
    assert_eq!(attrs.mime, before.mime);
    assert_eq!(attrs.hash, before.hash);
    assert_eq!(attrs.size, before.size);
}

#[test]
fn relocating_to_the_root_leaves_no_parent() {
    let mut attrs = Attributes::new("deep/down/a.txt", 1, jiff::Timestamp::UNIX_EPOCH);
    attrs.relocate("a.txt");
    assert_eq!(attrs.parent, "");
    assert_eq!(attrs.relative_path(), "a.txt");
}

#[test]
fn ancestors_are_every_directory_above_a_path_shallowest_first() {
    use crate::snapshot::ancestors;
    assert_eq!(ancestors("a/b/c.mp4"), ["a", "a/b"]);
    assert_eq!(ancestors("c.mp4"), Vec::<String>::new());
    assert_eq!(ancestors("a/b/c/d"), ["a", "a/b", "a/b/c"]);
}

#[test]
fn the_reserved_directory_is_itself_and_everything_under_it() {
    use crate::snapshot::is_reserved;
    assert!(is_reserved(".tungstate"));
    assert!(is_reserved(".tungstate/policy.toml"));
    assert!(is_reserved(".tungstate/deep/down.toml"));
    // Not a prefix match on the string: a sibling that merely starts the same
    // way is an ordinary file.
    assert!(!is_reserved(".tungstate-quarantine"));
    assert!(!is_reserved(".tungstaterc"));
}

#[test]
fn a_case_insensitive_snapshot_folds_names_together() {
    use crate::snapshot::Snapshot;
    use std::collections::BTreeSet;
    let entries = vec![Attributes::new("A.JPG", 1, jiff::Timestamp::UNIX_EPOCH)];
    let sensitive = Snapshot::new(
        jiff::Timestamp::UNIX_EPOCH,
        entries.clone(),
        BTreeSet::new(),
        true,
    );
    let folded = Snapshot::new(jiff::Timestamp::UNIX_EPOCH, entries, BTreeSet::new(), false);
    assert_ne!(sensitive.key("A.JPG"), sensitive.key("a.jpg"));
    assert_eq!(folded.key("A.JPG"), folded.key("a.jpg"));
}
