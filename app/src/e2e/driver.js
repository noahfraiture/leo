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

const openSettings = async () => {
    (await waitFor(() => Array.from(document.querySelectorAll("a"))
        .find((link) => link.textContent.trim() === "Settings"), "Settings navigation")).click();
    await waitFor(() => document.getElementById("application-settings-title"), "settings form");
};

const configureExercise = async (twoCriteria = false) => {
    await openAnalysis();
    const refresh = await waitFor(() => enabled("session-refresh"), "session refresh available");
    const completedBefore = refresh.dataset.refreshCompleted;
    refresh.click();
    await waitFor(() => {
        const current = enabled("session-refresh");
        return current && current.dataset.refreshCompleted !== completedBefore;
    }, "background session refresh acknowledgement");
    if (document.body.innerText.includes("Session list could not be refreshed")) {
        throw new Error("Background session refresh failed");
    }
    await waitFor(() => document.querySelector('button[aria-label^="Session "]'), "completed recording retained after refresh");
    (await waitFor(() => document.getElementById("analysis-configure-tab"), "configuration tab")).click();
    (await waitFor(() => document.getElementById("exercise-create"), "create exercise")).click();
    await input(await waitFor(() => document.getElementById("exercise-name"), "exercise name"), "Operator workflow test");
    await input(document.getElementById("exercise-pasted-checklist"), twoCriteria
        ? "Keep movement controlled\nKeep the work area visible"
        : "Keep movement controlled");
    (await waitFor(() => enabled("exercise-add-pasted"), "pasted checklist ready")).click();
    await input(document.getElementById("exercise-context"), "Indoor workbench with the operator visible from two cameras.");
    const order = document.getElementById("exercise-order");
    order.value = "explicit"; order.dispatchEvent(new Event("change", { bubbles: true }));
    (await waitFor(() => button("3. Clarify"), "clarification step")).click();
    (await waitFor(() => button("Format and review"), "formatting action")).click();
    (await waitFor(() => document.getElementById("exercise-apply-proposal"), "formatting proposal")).click();
    await waitFor(() => !document.getElementById("exercise-apply-proposal"), "proposal applied to draft");
    await input(await waitFor(() => document.querySelector('textarea[aria-label="Step 1"]'), "operator step editor"), "Keep movement controlled. Pauses are allowed.");
    await input(document.querySelector('section[aria-label="Operator clarification answers"] textarea'), "Pauses are allowed.");
    (await waitFor(() => button("4. Confirm"), "confirmation step")).click();
    (await waitFor(() => button("I reviewed the final wording"), "explicit reconciliation")).click();
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

const queuedCriterionControls = async (rows) => {
    await input(document.getElementById("analysis-correction-reason"), "First criterion draft");
    const oldMark = await waitFor(() => enabled("analysis-mark-reviewed"), "first criterion review action");
    const oldSave = await waitFor(() => enabled("analysis-save-correction"), "first criterion save action");
    rows[1].click();
    await waitFor(() => document.getElementById("analysis-correction-reason")?.value === "", "second criterion's own draft");
    await input(document.getElementById("analysis-correction-reason"), "Second criterion draft");
    oldMark.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await sleep(100);
    if (!document.body.innerText.includes("0 of 2 criteria reviewed")) {
        throw new Error("An old review action marked a different criterion");
    }
    oldSave.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await sleep(100);
    if (document.body.innerText.includes("Saved correction history")
        || document.getElementById("analysis-correction-reason").value !== "Second criterion draft") {
        throw new Error("An old save action consumed a different criterion's draft");
    }
    rows[0].click();
    await waitFor(() => document.getElementById("analysis-correction-reason")?.value === "First criterion draft", "first criterion's retained draft");
};

const queuedCorrectionCitationRemoval = async (previousAttach) => {
    const picker = document.querySelector('section[aria-label="Candidate recording playback"] select');
    const another = Array.from(picker.options).find((option) => option.value && option.value !== picker.value);
    if (!another) throw new Error("The citation removal fixture needs two candidate timestamps");
    picker.value = another.value;
    picker.dispatchEvent(new Event("change", { bubbles: true }));
    const attach = await waitFor(() => button("Attach selected timestamp")?.disabled === false
        ? button("Attach selected timestamp") : null, "attach another correction citation");
    previousAttach.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await sleep(100);
    if (document.querySelectorAll('[aria-label^="Remove correction citation"]').length !== 1) {
        throw new Error("An old Attach action attached the replacement timestamp");
    }
    attach.click();
    const removals = await waitFor(() => {
        const buttons = document.querySelectorAll('[aria-label^="Remove correction citation"]');
        return buttons.length === 2 ? buttons : null;
    }, "two attached correction citations");
    const oldRemove = removals[0];
    oldRemove.click();
    await waitFor(() => document.querySelectorAll('[aria-label^="Remove correction citation"]').length === 1, "first correction citation removed");
    oldRemove.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await sleep(100);
    if (document.querySelectorAll('[aria-label^="Remove correction citation"]').length !== 1) {
        throw new Error("A late citation removal deleted the surviving timestamp");
    }
};

const inspectAndCorrect = async (playVideo) => {
    (await waitForCompletedAnalysis()).click();
    const criteria = Array.from(document.querySelectorAll('[data-assessment-criterion]'));
    if (criteria.length === 2) await queuedCriterionControls(criteria);
    (await waitFor(() => document.querySelector('[data-evidence-id]'), "candidate citation")).click();
    if (playVideo) {
        const video = await waitFor(() => {
            const candidate = document.querySelector('video[id^="analysis-evidence-video-"]');
            return candidate?.readyState >= 2 && candidate.videoWidth > 0 ? candidate : null;
        }, "decoded candidate footage", 45000);
        video.scrollIntoView({ block: "center", behavior: "instant" });
        await sleep(100);
        const rect = video.getBoundingClientRect();
        if (rect.bottom <= 0 || rect.top >= innerHeight) throw new Error("Candidate player is outside the viewport");
        const before = video.currentTime;
        try {
            await video.play();
        } catch (error) {
            throw new Error(`Candidate playback rejected: ${error.name}: ${error.message}; ${JSON.stringify({
                visibility: document.visibilityState, focused: document.hasFocus(),
                readyState: video.readyState, width: video.videoWidth, height: video.videoHeight,
                paused: video.paused, muted: video.muted, duration: video.duration,
                currentTime: video.currentTime, connected: video.isConnected,
                mediaError: video.error?.code ?? null,
                userActivation: navigator.userActivation ? {
                    active: navigator.userActivation.isActive, previous: navigator.userActivation.hasBeenActive,
                } : null,
            })}`);
        }
        await waitFor(() => video.currentTime > before + 0.1, "actual video playback");
        video.pause();
        const target = Math.max(0, video.duration / 2);
        video.currentTime = target;
        await waitFor(() => !video.seeking && Math.abs(video.currentTime - target) < 0.2, "video seek");
        const decoded = await waitForVideoPixels(video);
        globalThis.leoPlaybackDiagnostics = decoded;
        if (!decoded.pixels?.nonBlack) throw new Error("Candidate player pixel check failed after waiting for decoded pixels: " + JSON.stringify(decoded));
    }
    (await waitFor(() => enabled("analysis-recheck"), "closer look availability")).click();
    await waitForCompletedAnalysis();
    await waitFor(() => document.body.innerText.includes("Independent closer look: Met"), "preserved independent result");
    if (!document.getElementById("analysis-recheck").disabled) throw new Error("Closer look allowance reset");
    const judgment = document.getElementById("analysis-correction-judgment");
    if (judgment.value !== "cannotVerify") throw new Error("Correction selector does not show the saved draft default");
    judgment.value = "cannotVerify";
    judgment.dispatchEvent(new Event("change", { bubbles: true }));
    const reason = document.getElementById("analysis-correction-reason");
    const correctionTyping = await typeAcrossDelayedRender(reason);
    await input(reason, `Operator review: ${correctionTyping}`);
    const attach = await waitFor(() => button("Attach selected timestamp"), "correction evidence");
    attach.click();
    await waitFor(() => document.querySelector('[aria-label^="Remove correction citation"]'), "attached correction citation");
    await queuedCorrectionCitationRemoval(attach);
    const consumedSave = await waitFor(() => enabled("analysis-save-correction"), "valid human correction");
    consumedSave.click();
    await waitFor(() => document.body.innerText.includes("Saved correction history"), "durable correction");
    await waitFor(() => document.getElementById("analysis-correction-reason")?.value === "", "saved correction reason cleared", 5000);
    await input(document.getElementById("analysis-correction-reason"), "Unsaved follow-up reason");
    consumedSave.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await sleep(200);
    if (document.getElementById("analysis-correction-reason")?.value !== "Unsaved follow-up reason") {
        throw new Error("A consumed Save action saved the replacement correction draft");
    }
    (await waitFor(() => enabled("analysis-mark-reviewed"), "criterion review")).click();
    await waitFor(() => document.body.innerText.includes(`1 of ${criteria.length} criteria reviewed`), "durable review mark");
    document.getElementById("analysis-mark-reviewed").dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await sleep(100);
    if (criteria.length === 2) {
        document.querySelectorAll('[data-assessment-criterion]')[1].click();
        await waitFor(() => document.getElementById("analysis-correction-reason")?.value === "Second criterion draft", "second criterion draft retained after first review");
        (await waitFor(() => enabled("analysis-mark-reviewed"), "second criterion review action")).click();
        await waitFor(() => document.body.innerText.includes("2 of 2 criteria reviewed"), "both criteria reviewed explicitly");
        document.querySelectorAll('[data-assessment-criterion]')[0].click();
        await waitFor(() => document.getElementById("analysis-correction-reason")?.value === "Unsaved follow-up reason", "first review draft restored");
    }
    const note = document.getElementById("student-note");
    const assessmentTyping = await typeAcrossDelayedRender(note);
    await input(note, `Overall review: ${assessmentTyping}`);
    (await waitFor(() => enabled("student-save"), "operator assessment ready")).click();
    await waitFor(() => document.body.innerText.includes("Saved operator assessment"), "saved human assessment");
    await waitFor(() => document.getElementById("student-save").disabled, "unchanged assessment cannot be saved twice", 5000);
    document.getElementById("student-save").dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await sleep(100);
    if (correctionTyping !== "abc" || assessmentTyping !== "abc") {
        throw new Error(`Older renders replaced human review typing: correction=${correctionTyping}; assessment=${assessmentTyping}`);
    }
};

const completeAnalysis = async (playVideo = true) => {
    await waitForLivePreviews();
    const publicPreview = await fetch("http://127.0.0.1:8889/camera-0/whep", {
        method: "POST",
        credentials: "omit",
        headers: { "Content-Type": "application/sdp" },
        body: "v=0\r\n",
    });
    if (publicPreview.status !== 401) {
        throw new Error(`Unauthenticated preview request was not rejected: ${publicPreview.status}`);
    }
    await startSession();
    await changeFirstCameraProfile();
    await sleep(3000);
    await openAnalysis();
    const refresh = await waitFor(() => enabled("session-refresh"), "active recording catalogue refresh");
    const refreshedBefore = refresh.dataset.refreshCompleted;
    refresh.click();
    await waitFor(() => {
        const current = enabled("session-refresh");
        return current && current.dataset.refreshCompleted !== refreshedBefore;
    }, "active recording catalogue refreshed");
    if (document.body.innerText.includes("Session list could not be refreshed")) {
        throw new Error("Active recording catalogue refresh failed");
    }
    if (document.body.innerText.includes("Recordings needing repair")) {
        throw new Error("The current healthy recording was listed as needing repair");
    }
    Array.from(document.querySelectorAll("a")).find((link) => link.textContent.trim() === "Monitor").click();
    await waitFor(() => document.body.innerText.includes("Session active"), "capture remained active across catalogue refresh");
    await stopSession();
    await configureExercise(!playVideo);
    await beginAnalysis();
    await inspectAndCorrect(playVideo);
    const history = document.getElementById("analysis-run-history");
    const firstRun = history.value;
    (await waitFor(() => document.getElementById("analysis-configure-tab"), "configuration tab")).click();
    (await waitFor(() => button("3. Clarify"), "edit final checklist")).click();
    await input(await waitFor(() => document.querySelector('textarea[aria-label="Step 1"]'), "existing criterion"), "Movement is visible. Pauses are allowed.");
    (await waitFor(() => button("4. Confirm"), "confirm revised configuration")).click();
    (await waitFor(() => enabled("exercise-publish"), "save edited configuration")).click();
    await waitFor(() => document.body.innerText.includes("Configuration saved and ready for analysis."), "revised configuration");
    await beginAnalysis();
    await waitFor(() => document.getElementById("analysis-run-history")?.value !== firstRun, "new run identity");
    await waitForCompletedAnalysis();
    if (document.getElementById("student-note").value !== "") throw new Error("New run inherited another run's assessment note");
    const chooser = document.getElementById("analysis-run-history");
    if (chooser.options.length !== 2) throw new Error("Rerun did not preserve both histories");
    chooser.value = firstRun;
    chooser.dispatchEvent(new Event("change", { bubbles: true }));
    (await waitFor(() => document.querySelector('[data-assessment-criterion]'), "first result")).click();
    await waitFor(() => document.body.innerText.includes("Keep movement controlled. Pauses are allowed.") && document.body.innerText.includes("Saved correction history"), "original checklist and correction");
    await waitFor(() => document.getElementById("student-note")?.value === "Overall review: abc"
        && document.getElementById("analysis-correction-reason")?.value === "Unsaved follow-up reason", "original run's saved note and unsaved correction draft restored");
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
    await openSettings();
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

const recoverExerciseLibrary = async () => {
    await openAnalysis();
    const choose = async (name) => {
        const option = await waitFor(() => Array.from(document.getElementById("exercise-selector")?.options ?? [])
            .find((option) => option.textContent === name), `saved exercise ${name}`);
        const select = document.getElementById("exercise-selector");
        select.value = option.value;
        select.dispatchEvent(new Event("change", { bubbles: true }));
        await waitFor(() => document.getElementById("exercise-name")?.value === name, `selected exercise ${name}`);
    };
    await choose("Healthy saved configuration");
    await waitFor(() => document.body.innerText.includes("Configurations needing repair"), "damaged folder warning");
    await waitFor(() => document.body.innerText.includes("Configuration saved and ready for analysis."), "healthy publication available");
    await input(document.getElementById("exercise-name"), "Healthy configuration updated");
    (await waitFor(() => button("4. Confirm"), "healthy confirmation step")).click();
    (await waitFor(() => enabled("exercise-publish"), "healthy publication remains writable")).click();
    await waitFor(() => document.body.innerText.includes("Configuration saved and ready for analysis."), "healthy update saved");
    await choose("Publication needs repair");
    await waitFor(() => document.body.innerText.includes("Saved publications could not load"), "damaged publication warning");
    await input(document.getElementById("exercise-name"), "Publication draft remains editable");
    await waitFor(() => document.body.innerText.includes("Unsaved changes. Save configuration before analysis."), "damaged publication draft edit reached application state");
    if (!document.body.innerText.includes("Saved publications could not load")) {
        throw new Error("Editing hid the publication storage warning");
    }
    return "ok\nHealthy exercises remained editable and publishable alongside damaged drafts and publications";
};

const queuedQuestionEvents = async () => {
    await openAnalysis();
    await waitFor(() => document.getElementById("exercise-name")?.value === "Queued question exercise", "saved question exercise");
    (await waitFor(() => button("3. Clarify"), "clarification step")).click();
    const rows = await waitFor(() => {
        const rows = document.querySelectorAll('section[aria-label="Operator clarification answers"] label');
        return rows.length === 2 ? rows : null;
    }, "two saved questions");
    const removed = rows[0];
    const remove = Array.from(removed.querySelectorAll("button")).find((candidate) => candidate.textContent.trim() === "Remove question");
    const oldAnswer = removed.querySelector("textarea");
    remove.click();
    remove.click();
    const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(oldAnswer), "value").set;
    setter.call(oldAnswer, "Late input for a removed question");
    oldAnswer.dispatchEvent(new Event("input", { bubbles: true }));
    const remaining = await waitFor(() => {
        const rows = document.querySelectorAll('section[aria-label="Operator clarification answers"] label');
        return rows.length === 1 && rows[0].textContent.includes("Second question") ? rows[0] : null;
    }, "second question retained after queued events");
    if (remaining.querySelector("textarea").value !== "Second answer") throw new Error("Queued input changed the wrong question");
    await input(remaining.querySelector("textarea"), "Updated second answer");
    (await waitFor(() => enabled("exercise-save-draft"), "question changes ready to save")).click();
    await waitFor(() => document.body.innerText.includes("Draft saved. Save configuration when the checklist and references are ready."), "question changes saved");
    return "ok\nQueued removal and input preserved the remaining question and its saved answer";
};

const preparationNavigation = async () => {
    await openAnalysis();
    (await waitFor(() => enabled("exercise-create"), "new configuration available")).click();
    await waitFor(() => document.getElementById("exercise-name")?.value === "New exercise", "new configuration");
    (await waitFor(() => button("2. References"), "reference step")).click();
    await waitFor(() => document.querySelector('section[aria-label="Reference library"]'), "reference library");
    const back = button("Back");
    back.click();
    back.click();
    await waitFor(() => document.getElementById("exercise-context") && Array.from(document.querySelectorAll("button"))
        .some((candidate) => candidate.textContent.trim() === "Back" && candidate.disabled), "first step after queued Back events");
    const next = button("Continue");
    for (let count = 0; count < 5; count++) next.click();
    await waitFor(() => document.querySelector('nav[aria-label="Exercise preparation steps"] button.tab-active')?.textContent.trim() === "4. Confirm" && Array.from(document.querySelectorAll("button"))
        .some((candidate) => candidate.textContent.trim() === "Continue" && candidate.disabled), "last step after queued Continue events");
    const returnToStart = button("Back");
    for (let count = 0; count < 5; count++) returnToStart.click();
    const name = await waitFor(() => document.getElementById("exercise-name"), "editable first step after returning");
    await input(name, "Navigation remains editable");
    (await waitFor(() => enabled("exercise-save-draft"), "navigation draft ready to save")).click();
    await waitFor(() => document.body.innerText.includes("Draft saved. Save configuration when the checklist and references are ready."), "navigation draft saved");
    return "ok\nQueued Back and Continue events remained within the preparation steps";
};

const typeAcrossDelayedRender = async (field, text = "abc") => {
    await input(field, "");
    const value = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(field), "value");
    const delayed = [];
    const interpreter = window.interpreter;
    const apply = interpreter.setAttributeInner;
    // Delay one host render before its equality check, matching a slower native bridge.
    interpreter.setAttributeInner = function(node, property, next, namespace) {
        if (node === field && (property === "value" || property === "initial_value")) {
            delayed.push({ value: next, apply: () => apply.call(this, node, property, next, namespace) });
        } else {
            apply.call(this, node, property, next, namespace);
        }
    };
    const edit = (text) => {
        value.set.call(field, text);
        field.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText" }));
    };
    try {
        edit(text[0]);
        await waitFor(() => delayed.length > 0, "first host input acknowledgement", 5000);
        edit(text.slice(0, 2));
        delayed.shift().apply();
        edit(field.value + text[2]);
        await waitFor(() => delayed.some((update) => update.value.endsWith(text[2])), "final input reached editor state", 5000);
    } finally {
        interpreter.setAttributeInner = apply;
        for (const update of delayed) update.apply();
    }
    return field.value;
};

const queuedProfileRemoval = async () => {
    await openSettings();
    const profile = await waitFor(() => document.getElementById("camera-1-sample-every"), "first camera profile");
    (await waitFor(() => button("Add monitoring profile"), "temporary monitoring profile")).click();
    const temporaryProfile = await waitFor(() => document.getElementById("monitoring-3-name"), "unassigned monitoring profile");
    const oldProfileRemove = temporaryProfile.parentElement.parentElement.querySelector("button");
    if (oldProfileRemove.disabled) throw new Error("Unassigned fixture profile cannot be removed");
    profile.value = "3";
    profile.dispatchEvent(new Event("change", { bubbles: true }));
    await waitFor(() => oldProfileRemove.disabled, "assigned profile is protected");
    oldProfileRemove.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await sleep(150);
    if (!document.getElementById("monitoring-3-name")) {
        throw new Error("A late Remove deleted the newly assigned monitoring profile");
    }
    profile.value = "1";
    profile.dispatchEvent(new Event("change", { bubbles: true }));
    await waitFor(() => !oldProfileRemove.disabled, "unassigned profile becomes removable again");
    oldProfileRemove.click();
    await waitFor(() => !document.getElementById("monitoring-3-name"), "explicitly remove the unassigned profile");
    (await waitFor(() => button("Save settings"), "profile changes ready to save")).click();
    await waitFor(() => document.body.innerText.includes("Settings saved."), "profile assignments saved");
    return "ok\nA queued Remove preserved the assigned profile; explicitly removing it after unassignment preserved both cameras' original profiles";
};

const settingsText = async () => {
    await openSettings();
    const failures = [];
    const check = async (field, label, text = "abc") => {
        const actual = await typeAcrossDelayedRender(field, text);
        if (actual !== text) failures.push(`${label}: ${actual}`);
        return actual;
    };
    const fieldFor = (label) => Array.from(document.querySelectorAll("label"))
        .find((element) => element.textContent.trim() === label)?.querySelector("input");
    const cameraButton = (id) => Array.from(document.querySelectorAll('section[aria-labelledby="settings-cameras-title"] button'))
        .find((candidate) => candidate.textContent.includes(`Camera ID ${id}`));
    const firstName = await check(await waitFor(() => document.getElementById("camera-1-name"), "first camera name"), "camera name");
    const url = document.getElementById("camera-1-rtsp-url");
    const originalUrl = url.value;
    await check(url, "camera URL");
    await input(url, originalUrl);
    cameraButton(2).click();
    const second = await waitFor(() => {
        const field = document.getElementById("camera-2-name");
        return field?.value === "Salon 2" ? field : null;
    }, "second camera's own name", 5000);
    await input(second, "Second camera retained");
    cameraButton(1).click();
    await waitFor(() => document.getElementById("camera-1-name")?.value === firstName, "first camera's retained edit", 5000);
    await check(document.getElementById("monitoring-1-name"), "monitoring name");
    await check(document.getElementById("monitoring-1-cadence"), "monitoring cadence", "123");
    await check(document.getElementById("settings-recorder-timeout"), "recorder timeout", "123");
    for (const label of ["Connection name", "Server address", "Display name", "Model identifier", "Set name"]) {
        const field = fieldFor(label);
        const original = field.value;
        await check(field, label);
        if (label === "Server address") await input(field, original);
    }
    await check(document.getElementById("provider-fixture-key"), "local fixture credential");
    for (const id of ["workflow-fixture-memory", "workflow-fixture-images", "workflow-fixture-tokens",
        "workflow-policy-memory", "workflow-policy-concurrent", "workflow-policy-per-model",
        "workflow-policy-timeout", "workflow-policy-span", "workflow-policy-overlap"]) {
        const field = document.getElementById(id);
        const original = field.value;
        await check(field, id, "123");
        if (!id.endsWith("-images") && !id.endsWith("-tokens")) await input(field, original);
    }
    if (failures.length) throw new Error(`Older renders replaced Settings typing: ${failures.join("; ")}`);
    (await waitFor(() => button("Save settings"), "typed settings ready to save")).click();
    await waitFor(() => document.body.innerText.includes("Settings saved."), "typed settings saved");
    await openAnalysis();
    await openSettings();
    await waitFor(() => document.getElementById("camera-1-name")?.value === "abc"
        && document.getElementById("monitoring-1-name")?.value === "abc"
        && document.getElementById("workflow-fixture-images")?.value === "123", "saved Settings drafts restored after navigation");
    return "ok\nSettings typing survived delayed rendering, camera selection restored its own draft, and edits were saved";
};

const queuedCameraEvents = async () => {
    await openSettings();
    const removedName = await waitFor(() => document.getElementById("camera-1-name"), "first camera selected");
    const remove = button("Remove camera");
    remove.click();
    remove.click();
    const setName = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(removedName), "value").set;
    setName.call(removedName, "Late input for the removed camera");
    removedName.dispatchEvent(new Event("input", { bubbles: true }));
    const remaining = await waitFor(() => {
        const field = document.getElementById("camera-2-name");
        return field?.value === "Salon 2" ? field : null;
    }, "second camera retained after queued removal and input", 5000);
    await input(remaining, "Surviving camera");
    (await waitFor(() => button("Save settings"), "remaining camera ready to save")).click();
    await waitFor(() => document.body.innerText.includes("Settings saved."), "remaining camera saved");
    return "ok\nQueued removal and input affected only the intended camera; the remaining camera stayed editable and saved";
};

const queuedModelSetEvents = async () => {
    await openSettings();
    const readers = () => Array.from(document.querySelectorAll("fieldset"))
        .find((field) => field.querySelector("legend")?.textContent === "Independent readers");
    const oldReader = await waitFor(() => readers()?.querySelector('input[type="checkbox"]'), "first legacy reader");
    const remove = Array.from(document.querySelectorAll("button"))
        .find((candidate) => candidate.textContent.trim() === "Remove model");
    remove.click();
    remove.click();
    const remaining = await waitFor(() => {
        const fields = readers()?.querySelectorAll('input[type="checkbox"]');
        return fields?.length === 1 && !fields[0].checked ? fields[0] : null;
    }, "remaining unassigned model");
    oldReader.checked = true;
    oldReader.dispatchEvent(new Event("change", { bubbles: true }));
    await sleep(150);
    if (readers().querySelector('input[type="checkbox"]').checked) {
        throw new Error("A removed model's checkbox assigned the surviving model");
    }
    remaining.checked = true;
    remaining.dispatchEvent(new Event("change", { bubbles: true }));
    (await waitFor(() => button("Save settings"), "legacy assignment ready to save")).click();
    await waitFor(() => document.body.innerText.includes("Settings saved."), "legacy assignment saved");
    return "ok\nLegacy reader changes remain attached to their model";
};


await (async () => {
    await verifyPreviewReaderLifecycle(previewReaderProgram);
    if (scenario === "complete-analysis") {
        dioxus.send(await completeAnalysis());
    } else if (scenario === "review-text") {
        dioxus.send(await completeAnalysis(false));
    } else if (scenario === "settings-text") {
        dioxus.send(await settingsText());
    } else if (scenario === "camera-events") {
        dioxus.send(await queuedCameraEvents());
    } else if (scenario === "profile-events") {
        dioxus.send(await queuedProfileRemoval());
    } else if (scenario === "model-set-events") {
        dioxus.send(await queuedModelSetEvents());
    } else if (scenario === "checklist-paste-events") {
        dioxus.send(await queuedChecklistPaste());
    } else if (scenario === "checklist-delete-events") {
        dioxus.send(await queuedChecklistDelete());
    } else if (scenario === "checklist-publish-events") {
        dioxus.send(await queuedChecklistPublication());
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
    } else if (scenario === "exercise-recovery") {
        dioxus.send(await recoverExerciseLibrary());
    } else if (scenario === "question-events") {
        dioxus.send(await queuedQuestionEvents());
    } else if (scenario === "preparation-navigation") {
        dioxus.send(await preparationNavigation());
    } else if (scenario === "rapid-exercise-input") {
        dioxus.send(await rapidExerciseInput());
    } else if (scenario === "checklist-replacement") {
        dioxus.send(await checklistReplacement());
    } else {
        throw new Error(`Unknown desktop E2E scenario: ${scenario}`);
    }
})().catch((error) => {
    dioxus.send(`error: ${error?.message ?? error}\n${error?.stack ?? ""}`);
});
