prepare-camera-video INPUT OUTPUT:
    ffmpeg -i {{quote(INPUT)}} -map 0:v:0 -an \
      -vf "fps=15,scale=1280:720:force_original_aspect_ratio=decrease,pad=1280:720:(ow-iw)/2:(oh-ih)/2" \
      -c:v libx264 -preset slow -crf 28 -profile:v baseline -level:v 3.1 \
      -pix_fmt yuv420p -bf 0 -g 15 -keyint_min 15 -sc_threshold 0 -flags +cgop \
      -movflags +faststart {{quote(OUTPUT)}}

camera-1:
    nix develop --command cargo run -p camera --bin camera -- --address 127.0.0.1:8080 --rtsp-address 127.0.0.1:8554 --video camera/fixtures/salon-1.mp4

camera-2:
    nix develop --command cargo run -p camera --bin camera -- --address 127.0.0.1:8081 --rtsp-address 127.0.0.1:8555 --video camera/fixtures/salon-2.mp4

vlc:
    nix develop --command vlc rtsp://127.0.0.1:8554/axis-media/media.amp

# First run opens Settings; later runs use saved platform settings after restart.
app:
    nix develop --command dx serve -p app --desktop

css:
    nix develop --command tailwindcss -i app/tailwind.css -o app/assets/tailwind.css

# Run all non-ignored tests, including feature-gated safety tests.
test-unit:
    nix develop --command cargo test --workspace --all-targets --all-features --locked

# Run every approved local media and mock desktop end-to-end check.
test-e2e:
    nix develop --command cargo test -p backend analysis::video::extractor::tests::extracts_fixture_frame_as_jpeg -- --ignored --exact
    nix develop --command cargo test -p backend analysis::session::tests::full_local_ffmpeg_and_mock_model_analysis_uses_pre_and_post_gap_segments -- --ignored --exact --nocapture
    nix develop --command cargo test -p camera --test rtsp_stream fixture_streams_h264_to_two_readers_and_stops_cleanly -- --ignored --exact
    nix develop --command cargo test -p camera --test rtsp_stream host_recorder_records_playable_mkv -- --ignored --exact --nocapture
    nix develop --command cargo test -p camera --test rtsp_stream host_recorder_reconnects_into_a_second_segment -- --ignored --exact --nocapture
    LEO_E2E_REAL_OPENAI=0 LEO_RUN_PAID_OPENAI_TEST=0 nix develop --command cargo test -p camera --features desktop-e2e --test desktop_e2e desktop_operator_flow_records_two_cameras_and_analyzes -- --ignored --exact --nocapture --test-threads=1

# Run all three real-OpenAI checks only after explicit caller opt-in.
test-paid:
    @test -z "${OPENAI_BASE_URL+x}" || (echo "unset OPENAI_BASE_URL; desktop paid validation targets OpenAI directly" >&2; exit 1)
    @test "${LEO_RUN_PAID_OPENAI_TEST:-}" = 1 && test "${LEO_E2E_REAL_OPENAI:-}" = 1 && test -n "${OPENAI_API_KEY:-}" && test -n "${ANALYSIS_MODEL:-}" || (echo "set both paid flags, OPENAI_API_KEY, and ANALYSIS_MODEL to run paid tests" >&2; exit 1)
    nix develop --command cargo test -p app --features paid-openai-evaluations openai_evaluations::natural_fixture_exercises_application_checkpoint_flow -- --ignored --exact --nocapture
    nix develop --command cargo test -p app --features paid-openai-evaluations openai_evaluations::controlled_visual_cases -- --ignored --exact --nocapture
    nix develop --command cargo test -p camera --features desktop-e2e --test desktop_e2e desktop_operator_flow_records_two_cameras_and_analyzes -- --ignored --exact --nocapture --test-threads=1
