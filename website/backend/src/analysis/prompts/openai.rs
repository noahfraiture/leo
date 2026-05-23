pub const VIDEO_ANALYSIS_INSTRUCTIONS: &str = "Analyze sampled video frames. Frames are chronological, may be chunked with overlap, and include video names and timestamps. Follow the user's request; use precise timestamps when they matter. Return plain text, not Markdown.";

pub fn chunk_evidence_request(
    user_prompt: &str,
    chunk_number: usize,
    chunk_count: usize,
    start_secs: f64,
    end_secs: f64,
) -> String {
    format!(
        "User request:\n{user_prompt}\n\nChunk {chunk_number} of {chunk_count} covers {start_secs:.3}s to {end_secs:.3}s.\nReturn concise evidence notes only: relevant observations, video names, timestamps, and uncertainty."
    )
}

pub fn frame_metadata(frame_number: usize, video_name: &str, timestamp_secs: f64) -> String {
    format!("Frame {frame_number}: video={video_name} timestamp={timestamp_secs:.3}s")
}

pub fn final_summary_request(user_prompt: &str, chunk_notes: &[String]) -> String {
    format!(
        "User request:\n{user_prompt}\n\nChunk notes:\n{}\n\nWrite the final answer in plain text, not Markdown. Use timestamps only when helpful. Do not mention chunking or overlap unless relevant.",
        chunk_notes
            .iter()
            .enumerate()
            .map(|(index, chunk)| format!("Chunk {}:\n{}", index + 1, chunk))
            .collect::<Vec<_>>()
            .join("\n\n")
    )
}
