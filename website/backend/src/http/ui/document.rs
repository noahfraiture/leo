use crate::http::router::AppState;
use hypertext::{Raw, prelude::*};

const STYLES: &str = include_str!("styles.generated.css");

/// Render a full HTML document around a UI route body.
pub fn document(_state: &AppState, title: &str, body: impl Renderable) -> impl Renderable {
    rsx! {
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <title>(title)</title>
                <style>(Raw::dangerously_create(STYLES))</style>
                <style>"[x-cloak] { display: none !important; }"</style>
                <script src="https://cdn.jsdelivr.net/npm/htmx.org@2.0.8/dist/htmx.min.js"></script>
                <script defer="defer" src="https://cdn.jsdelivr.net/npm/alpinejs@3.x.x/dist/cdn.min.js"></script>
                <script>
                    (Raw::dangerously_create(
                        r##"
                        document.addEventListener("alpine:init", () => {
                            Alpine.data("chunkedVideoUpload", () => ({
                                uploading: false,
                                status: "",
                                error: "",
                                maxChunkAttempts: 4,
                                async upload() {
                                    const file = this.$refs.video.files[0];
                                    if (!file) {
                                        return;
                                    }

                                    this.uploading = true;
                                    this.status = "Preparing upload";
                                    this.error = "";
                                    let uploadId = null;

                                    try {
                                        const start = await fetch("/videos/uploads", {
                                            method: "POST",
                                            headers: { "Content-Type": "application/json" },
                                            body: JSON.stringify({ filename: file.name, size: file.size }),
                                        });
                                        if (!start.ok) {
                                            throw new Error(await start.text());
                                        }

                                        const session = await start.json();
                                        uploadId = session.upload_id;
                                        const chunkSize = session.chunk_size;
                                        const totalChunks = Math.ceil(file.size / chunkSize);

                                        for (let index = 0; index < totalChunks; index += 1) {
                                            const startByte = index * chunkSize;
                                            const endByte = Math.min(file.size, startByte + chunkSize);
                                            const chunk = file.slice(startByte, endByte);

                                            this.status = `Uploading ${index + 1} / ${totalChunks} chunks`;
                                            await this.uploadChunkWithRetry(uploadId, index, chunk, totalChunks);
                                        }

                                        this.status = "Finalizing upload";
                                        const complete = await fetch(
                                            `/videos/uploads/${encodeURIComponent(uploadId)}/complete`,
                                            {
                                                method: "POST",
                                                headers: { "HX-Request": "true" },
                                            },
                                        );
                                        if (!complete.ok) {
                                            throw new Error(await complete.text());
                                        }

                                        const html = await complete.text();
                                        const workspace = document.querySelector("#video-workspace");
                                        if (workspace) {
                                            const template = document.createElement("template");
                                            template.innerHTML = html.trim();
                                            const nextWorkspace = template.content.firstElementChild;
                                            if (nextWorkspace) {
                                                workspace.replaceWith(nextWorkspace);
                                                Alpine.initTree(nextWorkspace);
                                            }
                                        }

                                        this.$refs.video.value = "";
                                        this.status = "Upload complete";
                                    } catch (error) {
                                        this.status = "";
                                        this.error = error.message || "Upload failed";
                                        if (uploadId) {
                                            fetch(`/videos/uploads/${encodeURIComponent(uploadId)}`, {
                                                method: "DELETE",
                                            }).catch(() => {});
                                        }
                                    } finally {
                                        this.uploading = false;
                                    }
                                },
                                async uploadChunkWithRetry(uploadId, index, chunk, totalChunks) {
                                    let lastError = "Upload failed";

                                    for (let attempt = 1; attempt <= this.maxChunkAttempts; attempt += 1) {
                                        if (attempt > 1) {
                                            this.status = `Retrying ${index + 1} / ${totalChunks} chunks`;
                                        }

                                        try {
                                            const response = await fetch(
                                                `/videos/uploads/${encodeURIComponent(uploadId)}/chunks/${index}`,
                                                {
                                                    method: "PUT",
                                                    headers: { "Content-Type": "application/octet-stream" },
                                                    body: chunk,
                                                },
                                            );
                                            if (response.ok) {
                                                return;
                                            }

                                            lastError = await response.text();
                                        } catch (error) {
                                            lastError = error.message || lastError;
                                        }

                                        if (attempt < this.maxChunkAttempts) {
                                            await new Promise((resolve) => setTimeout(resolve, attempt * 1000));
                                        }
                                    }

                                    throw new Error(lastError);
                                },
                            }));

                            Alpine.data("videoPlayer", () => ({
                                selectedVideo: "",
                                init() {
                                    this.selectedVideo = this.$el.dataset.selectedVideo || "";
                                },
                            }));
                        });
                        "##,
                    ))
                </script>
            </head>
            <body class="min-h-screen bg-base-100 text-base-content">
                (body)
            </body>
        </html>
    }
}

/// Default HTMX fallback body for routes that intentionally do not serve
/// fragments.
pub fn not_found_fragment() -> &'static str {
    "Not Found"
}
