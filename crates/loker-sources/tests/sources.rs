use loker_sources::{SOURCES, Source};

#[test]
fn only_boards_that_list_nothing_but_remote_jobs_are_remote_only() {
    let cases = [
        ("himalayas", true),
        ("jobicy", true),
        ("remoteok", true),
        ("weworkremotely", true),
        ("workable", true),
        ("ycombinator", false),
        ("greenhouse:acme", false),
        ("ashby:acme", false),
        ("lever:acme", false),
    ];
    assert_eq!(cases.len(), SOURCES.len(), "a row for every source");
    for (id, remote_only) in cases {
        assert_eq!(
            Source::parse(id).unwrap().remote_only(),
            remote_only,
            "{id}: a remote-only board lets its descriptions name places for unknown-workplace jobs"
        );
    }
}
