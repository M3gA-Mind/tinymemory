use super::*;

#[test]
fn openhuman_home_path_cases_map_to_home_marker() {
    for (input, expected) in [
        (
            "open /Users/oxoxdev/work/openhuman",
            "open <HOME>/work/openhuman",
        ),
        (r"C:\Users\oxoxdev\work\openhuman", r"<HOME>\work\openhuman"),
        ("C:/Users/oxoxdev/work/openhuman", "<HOME>/work/openhuman"),
        ("/home/jane/project", "<HOME>/project"),
        (r"c:\users\alice\work", r"<HOME>\work"),
        ("/HOME/alice/work", "<HOME>/work"),
        ("/Home/alice/work", "<HOME>/work"),
        (
            "from /Users/alice/a.txt to /Users/bob/b.txt",
            "from <HOME>/a.txt to <HOME>/b.txt",
        ),
        ("/Users/erin", "<HOME>"),
        (
            "open /home/carol/report.md and C:\\Users\\dave\\x.txt",
            "open <HOME>/report.md and <HOME>\\x.txt",
        ),
    ] {
        assert_eq!(redact_home_paths(input), expected, "input={input:?}");
    }
}

#[test]
fn tilde_and_unc_home_paths_are_scrubbed() {
    for (input, expected) in [
        ("~/Documents/report.txt", "<HOME>/Documents/report.txt"),
        ("~alice/Documents/report.txt", "<HOME>/Documents/report.txt"),
        (
            r"\\server\Users\alice\Documents\report.txt",
            r"<HOME>\Documents\report.txt",
        ),
        (r"\\server\home\alice\Documents", r"<HOME>\Documents"),
    ] {
        assert_eq!(redact_home_paths(input), expected, "input={input:?}");
    }
}

#[test]
fn unrelated_absolute_paths_and_non_paths_are_unchanged() {
    for input in [
        "/var/log/openhuman.log",
        "/opt/users/alice/data",
        r"D:\Projects\openhuman",
        "hello world",
        "prefix/home/alice/file",
        "项目/home/alice/file",
        "/home/",
        "/Users/",
        r"\server\Users\alice\data",
    ] {
        assert_eq!(redact_home_paths(input), input);
    }
}

#[test]
fn unicode_and_path_boundaries_are_preserved() {
    assert_eq!(
        redact_home_paths("📁 /Users/álîçé/文書/報告.txt and /home/用户/项目"),
        "📁 <HOME>/文書/報告.txt and <HOME>/项目"
    );
    assert_eq!(
        redact_home_paths("x=/home/a/one; y=~/two"),
        "x=<HOME>/one; y=<HOME>/two"
    );
}

#[test]
fn prose_after_bare_unix_windows_and_unc_homes_is_preserved() {
    assert_home_path_cases(&[
        ("open /Users/alice now", "open <HOME> now"),
        ("/home/alice and /home/bob/docs", "<HOME> and <HOME>/docs"),
        (
            r"C:\Users\alice and D:\Projects\report.txt",
            r"<HOME> and D:\Projects\report.txt",
        ),
        (r"\\server\Users\alice and more", r"<HOME> and more"),
    ]);
}

#[test]
fn punctuation_around_bare_home_paths_is_preserved() {
    assert_home_path_cases(&[
        ("(/Users/alice)", "(<HOME>)"),
        ("'/home/alice'", "'<HOME>'"),
        (r#""C:\Users\alice""#, r#""<HOME>""#),
        (r"\\server\Users\alice)", r"<HOME>)"),
        ("/Users/alice.", "<HOME>."),
        (
            "from /home/alice,/home/bob; done",
            "from <HOME>,<HOME>; done",
        ),
    ]);
}

#[test]
fn tilde_home_tokens_keep_boundaries_and_ignore_relative_dot_path() {
    assert_home_path_cases(&[
        ("~", "<HOME>"),
        ("~/docs", "<HOME>/docs"),
        ("~alice now", "<HOME> now"),
        ("~alice.smith/notes", "<HOME>/notes"),
        ("~alice.", "<HOME>."),
        ("(~)", "(<HOME>)"),
        ("~.", "~."),
        ("~.config/settings", "~.config/settings"),
    ]);
}

#[test]
fn dots_and_hyphens_inside_username_tokens_are_scrubbed() {
    assert_home_path_cases(&[
        (
            "/Users/alice.smith/docs and /home/bob-jones/work",
            "<HOME>/docs and <HOME>/work",
        ),
        (
            r"C:\Users\alice.smith\docs and \\server\Users\bob-jones\work",
            r"<HOME>\docs and <HOME>\work",
        ),
    ]);
}

#[test]
fn quoted_windows_and_unc_profiles_with_spaces_are_fully_scrubbed() {
    assert_home_path_cases(&[
        (
            r#""C:\Users\Jane Doe\Documents\report.txt""#,
            r#""<HOME>\Documents\report.txt""#,
        ),
        (
            r#""C:\Users\Mary-Jane Smith\Documents""#,
            r#""<HOME>\Documents""#,
        ),
        (r#"'C:\Users\Jane Doe'"#, r#"'<HOME>'"#),
        (
            r#""\\server\Users\Jane Doe\Documents""#,
            r#""<HOME>\Documents""#,
        ),
        (r#"'\\server\Users\Jane Doe'"#, r#"'<HOME>'"#),
        (
            r#""D:\Projects\Jane Doe\report.txt""#,
            r#""D:\Projects\Jane Doe\report.txt""#,
        ),
    ]);
}

#[test]
fn unquoted_windows_profiles_with_spaces_are_bounded_by_their_descendants() {
    assert_home_path_cases(&[
        (r"C:\Users\Jane Doe\Documents", r"<HOME>\Documents"),
        (r"\\server\Users\Jane Doe\Documents", r"<HOME>\Documents"),
        (r#""C:\Users\Jane Doe\Documents""#, r#""<HOME>\Documents""#),
        (
            r#""\\server\Users\Jane Doe\Documents""#,
            r#""<HOME>\Documents""#,
        ),
        (
            r"open C:\Users\alice and D:\Projects\report.txt",
            r"open <HOME> and D:\Projects\report.txt",
        ),
        (
            r"C:\Users\Jane Doe\Documents and /home/bob/docs",
            r"<HOME>\Documents and <HOME>/docs",
        ),
        (
            r"\\server\Users\Jane Doe\Documents and \\\other\Users\Bob\docs",
            r"<HOME>\Documents and <HOME>\docs",
        ),
    ]);
}

#[test]
fn legacy_home_components_with_symbols_are_redacted_through_boundaries() {
    assert_home_path_cases(&[
        ("/Users/O'Connor/Documents", "<HOME>/Documents"),
        ("/home/alice+dev/work", "<HOME>/work"),
        ("/Users/service$/Documents", "<HOME>/Documents"),
        ("/Users/alice\u{200d}dev/Documents", "<HOME>/Documents"),
        (r#""C:\Users\O'Connor\Documents""#, r#""<HOME>\Documents""#),
        (r"C:\Users\alice+dev\Documents", r"<HOME>\Documents"),
        (r"C:\Users\service$\Documents", r"<HOME>\Documents"),
        ("/home/$service/work", "<HOME>/work"),
        ("/home/$service", "<HOME>"),
        ("/Users/alice+dev", "<HOME>"),
        ("open /Users/O'Connor now", "open <HOME> now"),
        ("/Users/O'Connor!", "<HOME>!"),
        ("/home/alice+dev?", "<HOME>?"),
    ]);
}

#[test]
fn decomposed_unicode_username_marks_and_following_letters_are_scrubbed() {
    assert_home_path_cases(&[
        ("/Users/Zo\u{0308}eSmith/Documents", "<HOME>/Documents"),
        ("/Users/Jose\u{0301}/Documents", "<HOME>/Documents"),
        ("/home/Zo\u{0308}eSmith/work", "<HOME>/work"),
        ("/home/क्षमा/work", "<HOME>/work"),
    ]);
}

fn assert_home_path_cases(cases: &[(&str, &str)]) {
    let mismatches = cases
        .iter()
        .filter_map(|(input, expected)| {
            let actual = redact_home_paths(input);
            (actual != *expected)
                .then(|| format!("input={input:?}, actual={actual:?}, expected={expected:?}"))
        })
        .collect::<Vec<_>>();
    assert!(
        mismatches.is_empty(),
        "home path mismatches:\n{}",
        mismatches.join("\n")
    );
}
