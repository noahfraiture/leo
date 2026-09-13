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

const enabled = (id) => {
    const element = document.getElementById(id);
    return element && !element.disabled ? element : null;
};

const input = async (element, value) => {
    const setter = Object.getOwnPropertyDescriptor(
        Object.getPrototypeOf(element),
        "value",
    ).set;
    setter.call(element, value);
    element.dispatchEvent(new Event("input", { bubbles: true }));
    await sleep(100);
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
    (await waitFor(() => Array.from(document.querySelectorAll("a"))
        .find((candidate) => candidate.textContent.trim() === "Analyze"), "Analyze navigation")).click();
    await waitFor(() => document.getElementById("exercise-create"), "exercise setup");
};

const configureExercise = async () => {
    await openAnalysis();
    (await waitFor(() => document.getElementById("analysis-configure-tab"), "configuration tab")).click();
    (await waitFor(() => document.getElementById("exercise-create"), "create exercise")).click();
    await input(await waitFor(() => document.getElementById("exercise-name"), "exercise name"), "Operator workflow test");
    await input(document.getElementById("exercise-pasted-checklist"), "Keep movement controlled");
    (await waitFor(() => enabled("exercise-add-pasted"), "pasted checklist ready")).click();
    (await waitFor(() => button("Format and review"), "formatting action")).click();
    (await waitFor(() => document.getElementById("exercise-apply-proposal"), "formatting proposal")).click();
    await waitFor(() => !document.getElementById("exercise-apply-proposal"), "proposal applied to draft");
    await input(await waitFor(() => document.querySelector('textarea[aria-label="Step 1"]'), "operator step editor"), "Keep movement controlled. Pauses are allowed.");
    (await waitFor(() => enabled("exercise-publish"), "valid exercise")).click();
    await waitFor(() => document.body.innerText.includes("Configuration saved and ready for analysis."), "published exercise");
};

const beginAnalysis = async () => {
    (await waitFor(() => document.getElementById("analysis-evaluate-tab"), "evaluation tab")).click();
    (await waitFor(() => document.querySelector('button[aria-label^="Session "]'), "completed recording")).click();
    (await waitFor(() => enabled("analysis-start"), "analysis start")).click();
};

const waitForCompletedAnalysis = async () => {
    await waitFor(() => document.querySelector('section[aria-label="Analysis run progress"]')?.innerText.includes("Ready for review"), "completed run", 120000);
    return waitFor(() => document.querySelector('[data-assessment-criterion]'), "rendered criterion assessment");
};

const inspectAndCorrect = async () => {
    (await waitForCompletedAnalysis()).click();
    (await waitFor(() => document.querySelector('[data-evidence-id]'), "candidate citation")).click();
    const video = await waitFor(() => {
        const candidate = document.querySelector('video[id^="analysis-evidence-video-"]');
        return candidate?.readyState >= 2 && candidate.videoWidth > 0 ? candidate : null;
    }, "decoded candidate footage", 45000);
    const before = video.currentTime;
    await video.play();
    await waitFor(() => video.currentTime > before + 0.1, "actual video playback");
    video.pause();
    const target = Math.max(0, video.duration / 2);
    video.currentTime = target;
    await waitFor(() => !video.seeking && Math.abs(video.currentTime - target) < 0.2, "video seek");
    const judgment = document.getElementById("analysis-correction-judgment");
    judgment.value = "cannotVerify";
    judgment.dispatchEvent(new Event("change", { bubbles: true }));
    await input(document.getElementById("analysis-correction-reason"), "Operator inspected the actual footage; the interpretation remains uncertain.");
    (await waitFor(() => button("Attach currently viewed timestamp"), "correction evidence")).click();
    await waitFor(() => document.querySelector('[aria-label^="Remove correction citation"]'), "attached correction citation");
    (await waitFor(() => enabled("analysis-save-correction"), "valid human correction")).click();
    await waitFor(() => document.body.innerText.includes("Saved correction history"), "durable correction");
};

const completeAnalysis = async () => {
    await waitForLivePreviews();
    await startSession();
    await changeFirstCameraProfile();
    await sleep(3000);
    await stopSession();
    await configureExercise();
    await beginAnalysis();
    await inspectAndCorrect();
    const history = document.getElementById("analysis-run-history");
    const firstRun = history.value;
    (await waitFor(() => document.getElementById("analysis-configure-tab"), "configuration tab")).click();
    await input(await waitFor(() => document.querySelector('textarea[aria-label="Step 1"]'), "existing criterion"), "Movement is visible. Pauses are allowed.");
    (await waitFor(() => enabled("exercise-publish"), "save edited configuration")).click();
    await waitFor(() => document.body.innerText.includes("Configuration saved and ready for analysis."), "revised configuration");
    await beginAnalysis();
    await waitFor(() => document.getElementById("analysis-run-history")?.value !== firstRun, "new run identity");
    await waitForCompletedAnalysis();
    const chooser = document.getElementById("analysis-run-history");
    if (chooser.options.length !== 2) throw new Error("Rerun did not preserve both histories");
    chooser.value = firstRun;
    chooser.dispatchEvent(new Event("change", { bubbles: true }));
    (await waitFor(() => document.querySelector('[data-assessment-criterion]'), "first result")).click();
    await waitFor(() => document.body.innerText.includes("Keep movement controlled. Pauses are allowed.") && document.body.innerText.includes("Saved correction history"), "original checklist and correction");
    return "ok\nWorkflow complete";
};

const recoverAnalysis = async () => {
    await waitForLivePreviews();
    await startSession();
    await sleep(4000);
    await stopSession();
    await configureExercise();
    await beginAnalysis();
    await waitFor(() => document.body.innerText.includes("Processing paused."), "failed provider request", 120000);
    const progress = await waitFor(() => {
        const element = document.querySelector('progress[aria-label^="Completed model requests:"]');
        return element && Number(element.value) > 0 && Number(element.value) < Number(element.max) ? element.getAttribute("aria-label") : null;
    }, "preserved independent results");
    (await waitFor(() => button("Resume saved run"), "resume action")).click();
    await waitForCompletedAnalysis();
    return `ok\n${progress}\nWorkflow resumed`;
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

// A deliberately opt-in 15-minute capture plus two short restart cycles.
// Analysis is omitted so model inference cannot compete with capture.
const recordingRehearsal = async () => {
    await waitForLivePreviews();
    await startSession();
    const started = Date.now();
    await changeFirstCameraProfile();
    await sleep(30000);
    (await waitFor(() => button("Exclude from analysis"), "participation control")).click();
    await sleep(10000);
    (await waitFor(() => button("Include in analysis"), "participation control")).click();
    await sleep(Math.max(0, 900000 - (Date.now() - started)));
    if (document.querySelectorAll('[aria-label$="recorder status: Recording"]').length !== 2) {
        throw new Error("Both recorders did not remain active through the rehearsal");
    }
    await stopSession();
    for (let cycle = 0; cycle < 2; cycle++) {
        await startSession();
        await changeFirstCameraProfile();
        await sleep(10000);
        await stopSession();
    }
    return "ok\n15-minute recording and two subsequent sessions completed";
};

const profileSettings = async (save) => {
    (await waitFor(() => Array.from(document.querySelectorAll("a"))
        .find((link) => link.textContent.trim() === "Settings"), "Settings navigation")).click();
    await waitFor(() => document.getElementById("application-settings-title"), "settings form");
    await waitFor(() => document.getElementById("settings-log-level")?.value === "info", "initial log-level selection");
    if (save) {
        (await waitFor(() => button("Add camera"), "add temporary camera")).click();
        await waitFor(() => document.querySelector('select[id^="camera-"][id$="-sample-every"]')?.value === "1", "initial camera profile selection");
        (await waitFor(() => button("Remove camera"), "remove temporary camera")).click();
        (await waitFor(() => button("Add monitoring profile"), "add monitoring profile")).click();
        await waitFor(() => document.getElementById("monitoring-3-name"), "new monitoring profile");
        await input(document.getElementById("monitoring-3-name"), "Evaluation slow");
        await input(document.getElementById("monitoring-3-cadence"), "2000");
        await input(document.getElementById("workflow-policy-memory"), "4096");
        await input(document.getElementById("workflow-policy-concurrent"), "3");
        await input(document.querySelector('[id^="workflow-"][id$="-images"]'), "24");
        (await waitFor(() => button("Save settings"), "save settings")).click();
        await waitFor(() => document.body.innerText.includes("Settings saved."), "saved confirmation");
    }
    for (const [id, expected] of Object.entries({"monitoring-3-name": "Evaluation slow",
        "monitoring-3-cadence": "2000", "workflow-policy-memory": "4096", "workflow-policy-concurrent": "3"})) {
        await waitFor(() => document.getElementById(id)?.value === expected, `persisted ${id}`);
    }
    await waitFor(() => document.querySelector('[id^="workflow-"][id$="-images"]')?.value === "24", "persisted model image capacity");
    return `ok\nprofiles ${save ? "saved" : "reloaded"} through the desktop settings form`;
};

await (async () => {
    if (scenario === "complete-analysis") {
        dioxus.send(await completeAnalysis());
    } else if (scenario === "analysis-recovery") {
        dioxus.send(await recoverAnalysis());
    } else if (scenario === "record-without-preview") {
        dioxus.send(await recordWithoutPreview());
    } else if (scenario === "recording-rehearsal") {
        dioxus.send(await recordingRehearsal());
    } else if (scenario === "save-profiles" || scenario === "reload-profiles") {
        dioxus.send(await profileSettings(scenario === "save-profiles"));
    } else if (scenario === "analyze-existing") {
        await configureExercise();
        await beginAnalysis();
        await waitForCompletedAnalysis();
        dioxus.send("ok\nExisting recording analyzed");
    } else {
        throw new Error(`Unknown desktop E2E scenario: ${scenario}`);
    }
})().catch((error) => {
    dioxus.send(`error: ${error?.message ?? error}\n${error?.stack ?? ""}`);
});
