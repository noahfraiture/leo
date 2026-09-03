const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

const waitFor = async (predicate, description, timeout = 30000) => {
    const started = Date.now();
    while (Date.now() - started < timeout) {
        const value = predicate();
        if (value) return value;
        await sleep(100);
    }
    const body = document.body?.innerText?.slice(0, 4000) ?? "document body unavailable";
    throw new Error(`Timed out waiting for ${description}. Body: ${body}`);
};

const button = (label) => Array.from(document.querySelectorAll("button"))
    .find((candidate) => candidate.textContent.trim() === label && !candidate.disabled);

const input = (element, value) => {
    const setter = Object.getOwnPropertyDescriptor(
        Object.getPrototypeOf(element),
        "value",
    ).set;
    setter.call(element, value);
    element.dispatchEvent(new Event("input", { bubbles: true }));
};

(async () => {
    await waitFor(() => ["camera-0-video", "camera-1-video"].every((id) => {
        const video = document.getElementById(id);
        const tracks = video?.srcObject?.getVideoTracks?.() ?? [];
        return video && video.videoWidth > 0 && tracks.some((track) => track.readyState === "live");
    }), "two live previews", 45000);

    (await waitFor(() => button("Start session"), "Start session button")).click();
    await waitFor(() => document.body.innerText.includes("Session active"), "active session", 45000);
    await waitFor(
        () => document.querySelectorAll('[aria-label$="recorder status: Recording"]').length === 2,
        "two recording statuses",
    );

    const cadence = await waitFor(
        () => document.getElementById("sampling-interval-1"),
        "camera one cadence input",
    );
    input(cadence, "2");
    cadence.closest("form").dispatchEvent(new Event("submit", {
        bubbles: true,
        cancelable: true,
    }));
    await sleep(3000);

    (await waitFor(() => button("Stop session"), "Stop session button")).click();
    await waitFor(() => document.body.innerText.includes("Session idle"), "idle session", 45000);

    const analyze = await waitFor(
        () => Array.from(document.querySelectorAll("a"))
            .find((candidate) => candidate.textContent.trim() === "Analyze"),
        "Analyze navigation",
    );
    analyze.click();
    await waitFor(() => document.getElementById("completed-sessions-title"), "completed sessions");

    let checklist = document.getElementById("analysis-checklist");
    if (!checklist) {
        const row = await waitFor(
            () => document.querySelector('button[aria-label^="Session "]'),
            "completed session row",
        );
        row.click();
        checklist = await waitFor(
            () => document.getElementById("analysis-checklist"),
            "analysis checklist",
        );
    }
    input(checklist, "Keep movement controlled");
    (await waitFor(() => button("Analyze"), "Analyze action")).click();

    await waitFor(
        () => document.querySelector('button[aria-label*="status: Complete"]'),
        "completed analysis",
        90000,
    );
    await waitFor(() => document.getElementById("analysis-results-title"), "analysis results");
    const renderedSummary = await waitFor(
        () => document.querySelector('[aria-labelledby="sequence-summary-title"] p')
            ?.textContent.trim() || null,
        "rendered sequence summary",
    );
    dioxus.send(`ok\n${renderedSummary}`);
})().catch((error) => {
    dioxus.send(`error: ${error?.stack ?? error}`);
});
