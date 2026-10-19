use super::*;

#[test]
fn openhuman_home_path_cases_map_to_home_marker() {
    for (input, expected) in [
        ("open /Users/oxoxdev/work/openhuman", "open <HOME>/work/openhuman"),
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
        (r"\\server\Users\alice\Documents\report.txt", r"<HOME>\Documents\report.txt"),
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
