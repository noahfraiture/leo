fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("--clip") {
        assert_eq!(
            args.len(),
            4,
            "--clip MP4 result.json initial-seek-ms required"
        );
        app::launch_clip_playback_probe(
            args[1].clone().into(),
            args[2].clone().into(),
            args[3].parse().expect("integer seek milliseconds"),
        );
    } else {
        assert_eq!(
            args.len(),
            2,
            "session directory and result JSON path required"
        );
        app::launch_playback_probe(args[0].clone().into(), args[1].clone().into());
    }
}
