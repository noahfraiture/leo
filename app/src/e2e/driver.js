const scenario = await dioxus.recv();
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

const waitForLivePreviews = () => waitFor(() => ["camera-0-video", "camera-1-video"].every((id) => {
    const video = document.getElementById(id);
    const tracks = video?.srcObject?.getVideoTracks?.() ?? [];
    return video && video.videoWidth > 0 && tracks.some((track) => track.readyState === "live");
}), "two live previews", 45000);

const startSession = async () => {
    (await waitFor(() => button("Start session"), "Start session button")).click();
    await waitFor(() => document.body.innerText.includes("Session active"), "active session", 45000);
    await waitFor(
        () => document.querySelectorAll('[aria-label$="recorder status: Recording"]').length === 2,
        "two recording statuses",
    );
};

const changeFirstCameraProfile = async () => {
    const select = await waitFor(() => document.getElementById("monitoring-profile-1"), "camera one monitoring selector");
    select.value = "1";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await sleep(100);
    (await waitFor(() => button("Apply to all cameras"), "bulk monitoring action")).click();
    await sleep(100);
    select.value = "2";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await sleep(100);
    (await waitFor(() => button("Apply to selected camera"), "individual monitoring action")).click();
};

const stopSession = async () => {
    (await waitFor(() => button("Stop session"), "Stop session button")).click();
    await waitFor(() => document.body.innerText.includes("Session idle"), "idle session", 45000);
};

const openAnalysis = async () => {
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
    return checklist;
};

const beginAnalysis = async () => {
    const checklist = await openAnalysis();
    input(checklist, "Keep movement controlled");
    (await waitFor(() => button("Analyze"), "Analyze action")).click();
};

const waitForCompletedAnalysis = async () => {
    await waitFor(
        () => document.querySelector('button[aria-label*="status: Complete"]'),
        "completed analysis",
        90000,
    );
    await waitFor(() => document.getElementById("analysis-results-title"), "analysis results");
    return waitFor(
        () => document.querySelector('[aria-labelledby="sequence-summary-title"] p')
            ?.textContent.trim() || null,
        "rendered sequence summary",
    );
};

const waitForFailedPartialAnalysis = async () => {
    await waitFor(
        () => document.querySelector('button[aria-label*="status: Failed"]'),
        "failed analysis",
        90000,
    );
    await waitFor(
        () => document.querySelector('[role="alert"]')?.textContent.trim() || null,
        "visible analysis error",
    );
    return waitFor(() => {
        const progress = document.querySelector('[aria-label^="Analysis progress: "]');
        const label = progress?.getAttribute("aria-label");
        const match = label?.match(/^Analysis progress: (\d+) of (\d+) batches$/);
        return match && Number(match[1]) > 0 && Number(match[1]) < Number(match[2])
            ? label
            : null;
    }, "saved partial analysis progress");
};

const completeAnalysis = async () => {
    await waitForLivePreviews();
    await startSession();
    await changeFirstCameraProfile();
    await sleep(3000);
    await stopSession();
    await beginAnalysis();
    const renderedSummary = await waitForCompletedAnalysis();
    return `ok\n${renderedSummary}`;
};

const recoverAnalysis = async () => {
    await waitForLivePreviews();
    await startSession();
    await sleep(4000);
    await stopSession();
    await beginAnalysis();
    const partialProgress = await waitForFailedPartialAnalysis();
    (await waitFor(() => button("Resume"), "Resume action")).click();
    const renderedSummary = await waitForCompletedAnalysis();
    return `ok\n${partialProgress}\n${renderedSummary}`;
};

const recordWithoutPreview = async () => {
    await waitFor(
        () => Array.from(document.querySelectorAll('[role="alert"]')).find((candidate) => {
            const message = candidate.textContent;
            return message.includes("Live preview is unavailable")
                && message.includes("free the preview ports");
        }),
        "preview failure guidance",
    );
    await startSession();
    await changeFirstCameraProfile();
    (await waitFor(() => button("Exclude from analysis"), "participation control")).click();
    await sleep(2000);
    dioxus.send("inject-metadata-failure");
    await dioxus.recv();
    const select = document.getElementById("monitoring-profile-1");
    select.value = "1";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await sleep(100);
    (await waitFor(() => button("Apply to all cameras"), "bulk metadata write")).click();
    await waitFor(() => document.body.innerText.includes("Last saved"), "last saved metadata warning");
    await sleep(3000);
    if (document.querySelectorAll('[aria-label$="recorder status: Recording"]').length !== 2) {
        throw new Error("Metadata failure interrupted camera recording");
    }
    await stopSession();
    await startSession();
    await sleep(3000);
    await stopSession();
    return "ok\nrecording continued across metadata failure and a second session";
};

await (async () => {
    if (scenario === "complete-analysis") {
        dioxus.send(await completeAnalysis());
    } else if (scenario === "analysis-recovery") {
        dioxus.send(await recoverAnalysis());
    } else if (scenario === "record-without-preview") {
        dioxus.send(await recordWithoutPreview());
    } else {
        throw new Error(`Unknown desktop E2E scenario: ${scenario}`);
    }
})().catch((error) => {
    dioxus.send(`error: ${error?.stack ?? error}`);
});
