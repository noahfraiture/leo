fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert_eq!(
        args.len(),
        2,
        "session directory and result JSON path required"
    );
    app::launch_playback_probe(args[0].clone().into(), args[1].clone().into());
}
