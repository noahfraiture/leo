// Native exercise editing checks share the driver helpers and disposable fixture.
const queuedChecklistPublication = async () => {
    await openAnalysis();
    await waitFor(() => document.getElementById("exercise-name")?.value === "Prepared text exercise", "prepared exercise");
    (await waitFor(() => button("4. Confirm"), "confirmation step")).click();
    const oldPublish = await waitFor(() => enabled("exercise-publish"), "unpublished clean draft can be confirmed");
    button("3. Clarify").click();
    await input(await waitFor(() => document.querySelector('textarea[aria-label="Step 1"]'), "editable final checklist"), "Changed after leaving confirmation.");
    if (!oldPublish.disabled) throw new Error("Publication fixture did not leave confirmation");
    oldPublish.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await sleep(200);
    if (document.body.innerText.includes("Configuration saved and ready for analysis.")
        || !document.body.innerText.includes("Unsaved changes. Save configuration before analysis.")) {
        throw new Error("A late Confirm published edits outside the confirmation step");
    }
    button("4. Confirm").click();
    (await waitFor(() => enabled("exercise-publish"), "explicit confirmation of new wording")).click();
    await waitFor(() => document.body.innerText.includes("Configuration saved and ready for analysis."), "new wording explicitly published");
    return "ok\nPublication requires the current confirmation step";
};

const rapidExerciseInput = async () => {
    await openAnalysis();
    await waitFor(() => document.getElementById("exercise-name")?.value === "Prepared text exercise", "prepared exercise");
    const failures = [];
    const check = async (field, label) => {
        const actual = await typeAcrossDelayedRender(field);
        if (actual !== "abc") failures.push(`${label}: ${actual}`);
    };
    for (const id of ["exercise-name", "exercise-context"]) {
        await check(await waitFor(() => document.getElementById(id), id), id);
    }
    await check(document.querySelector('textarea[aria-label="Original step 1"]'), "original source");
    (await waitFor(() => button("3. Clarify"), "clarification step")).click();
    await check(await waitFor(() => document.querySelector('section[aria-label="Operator clarification answers"] textarea'), "clarification answer"), "clarification answer");
    (await waitFor(() => button("2. References"), "reference step")).click();
    const references = await waitFor(() => {
        const fields = document.querySelectorAll('article[data-reference-id] input:not([type="checkbox"])');
        return fields.length === 3 ? fields : null;
    }, "three reference label fields");
    for (const [index, field] of Array.from(references).entries()) await check(field, `reference label ${index + 1}`);
    (await waitFor(() => button("3. Clarify"), "checklist paste step")).click();
    const pasted = await waitFor(() => document.getElementById("exercise-pasted-checklist"), "checklist paste field");
    await check(pasted, "pasted checklist");
    if (failures.length) throw new Error(`Older renders replaced newer typing: ${failures.join("; ")}`);
    (await waitFor(() => enabled("exercise-add-pasted"), "typed pasted step ready to add")).click();
    const setPasted = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(pasted), "value").set;
    setPasted.call(pasted, "Late input for a consumed paste");
    pasted.dispatchEvent(new Event("input", { bubbles: true }));
    await waitFor(() => document.querySelectorAll("div[data-criterion-id]").length === 2, "pasted step added");
    await waitFor(() => document.getElementById("exercise-pasted-checklist")?.value === "", "paste field cleared after Add", 5000);
    (await waitFor(() => enabled("exercise-save-draft"), "typed draft ready to save")).click();
    await waitFor(() => document.body.innerText.includes("Draft saved. Save configuration when the checklist and references are ready."), "typed draft saved");
    return "ok\nDelayed host rendering preserved every typed character in saved operator text";
};

const checklistReplacement = async () => {
    await openAnalysis();
    await waitFor(() => document.getElementById("exercise-name")?.value === "Prepared text exercise", "prepared exercise");
    (await waitFor(() => button("3. Clarify"), "final checklist step")).click();
    const field = await waitFor(() => document.querySelector('textarea[aria-label="Step 1"]'), "final checklist wording");
    const typed = await typeAcrossDelayedRender(field);
    const format = await waitFor(() => enabled("exercise-format"), "checklist review available");
    const model = document.getElementById("exercise-formatter-model");
    model.value = "selected-model";
    model.dispatchEvent(new Event("change", { bubbles: true }));
    format.click();
    const suggestion = await waitFor(() => document.querySelector('section[aria-label="Checklist formatting assistance"] textarea'), "returned suggestion");
    const proposedTyping = await typeAcrossDelayedRender(suggestion);
    const oldApply = document.getElementById("exercise-apply-proposal");
    const oldDismiss = button("Dismiss proposal");
    (await waitFor(() => enabled("exercise-format"), "another explicit review available")).click();
    const replacement = await waitFor(() => {
        const field = document.querySelector('section[aria-label="Checklist formatting assistance"] textarea');
        return field?.value === "Place the item in the case." ? field : null;
    }, "new proposal replaces the edited suggestion", 5000);
    oldDismiss.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await sleep(100);
    if (!document.getElementById("exercise-apply-proposal")) throw new Error("An old Dismiss consumed the replacement proposal");
    oldApply.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await sleep(100);
    if (!document.getElementById("exercise-apply-proposal") || document.querySelector('textarea[aria-label="Step 1"]').value !== typed) {
        throw new Error("An old Apply consumed the replacement proposal");
    }
    await input(replacement, "Applied proposal wording.");
    (await waitFor(() => enabled("exercise-apply-proposal"), "explicit proposal application")).click();
    await waitFor(() => !document.getElementById("exercise-apply-proposal"), "proposal applied");
    await waitFor(() => document.querySelector('textarea[aria-label="Step 1"]')?.value === "Applied proposal wording.", "applied proposal displayed in the final editor", 5000);
    (await waitFor(() => enabled("exercise-save-draft"), "applied draft ready to save")).click();
    await waitFor(() => document.body.innerText.includes("Draft saved. Save configuration when the checklist and references are ready."), "applied wording saved");
    const oldDraftSave = document.getElementById("exercise-save-draft");
    (await waitFor(() => button("4. Confirm"), "confirm applied wording")).click();
    const oldPublish = await waitFor(() => enabled("exercise-publish"), "clean draft can be confirmed");
    oldPublish.click();
    await waitFor(() => document.body.innerText.includes("Configuration saved and ready for analysis."), "applied wording published");
    oldDraftSave.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await sleep(250);
    if (!document.body.innerText.includes("Configuration saved and ready for analysis.")) {
        throw new Error("A late unchanged draft save invalidated the confirmed configuration");
    }
    oldPublish.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await sleep(250);
    await waitFor(() => document.body.innerText.includes("Configuration saved and ready for analysis."), "confirmed configuration stays current");
    if (typed !== "abc") throw new Error(`An older render replaced final-checklist typing: ${typed}`);
    if (proposedTyping !== "abc") throw new Error(`An older render replaced proposal typing: ${proposedTyping}`);
    return "ok\nChecklist and proposal typing survived delayed rendering, Apply replaced the final wording, and late unchanged saves preserved its publication";
};

const queuedChecklistPaste = async () => {
    await openAnalysis();
    await waitFor(() => document.getElementById("exercise-name")?.value === "Prepared text exercise", "prepared exercise");
    (await waitFor(() => button("3. Clarify"), "final checklist step")).click();
    await input(await waitFor(() => document.getElementById("exercise-pasted-checklist"), "paste field"), "First added step");
    const oldAdd = await waitFor(() => enabled("exercise-add-pasted"), "first pasted step ready");
    oldAdd.click();
    await waitFor(() => document.querySelectorAll("div[data-criterion-id]").length === 2, "first pasted step consumed");
    await input(document.getElementById("exercise-pasted-checklist"), "Second added step");
    await waitFor(() => enabled("exercise-add-pasted"), "replacement paste ready");
    oldAdd.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await sleep(150);
    if (document.querySelectorAll("div[data-criterion-id]").length !== 2
        || document.getElementById("exercise-pasted-checklist")?.value !== "Second added step") {
        throw new Error("An old Add consumed the replacement paste buffer");
    }
    document.getElementById("exercise-add-pasted").click();
    await waitFor(() => document.querySelectorAll("div[data-criterion-id]").length === 3, "replacement explicitly added");
    (await waitFor(() => enabled("exercise-save-draft"), "paste result ready to save")).click();
    await waitFor(() => document.body.innerText.includes("Draft saved. Save configuration when the checklist and references are ready."), "paste result saved");
    return "ok\nA consumed paste control cannot consume its replacement buffer";
};

const queuedChecklistDelete = async () => {
    await openAnalysis();
    await waitFor(() => document.getElementById("exercise-name")?.value === "Prepared text exercise", "prepared exercise");
    (await waitFor(() => button("3. Clarify"), "final checklist step")).click();
    (await waitFor(() => document.querySelector('button[aria-label="Delete step 1"]'), "first delete")).click();
    const oldConfirm = await waitFor(() => button("Delete step and links"), "first confirmation");
    const oldCancel = button("Cancel");
    document.querySelector('button[aria-label="Delete step 2"]').click();
    await waitFor(() => document.querySelector('[role="alert"]')?.innerText.includes("0 reference images"), "second confirmation");
    oldCancel.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await sleep(100);
    if (!button("Delete step and links")) throw new Error("An old Cancel dismissed a different step confirmation");
    oldConfirm.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await sleep(100);
    if (document.querySelectorAll("div[data-criterion-id]").length !== 2 || !button("Delete step and links")) {
        throw new Error("An old confirmation deleted a different checklist step");
    }
    button("Delete step and links").click();
    await waitFor(() => document.querySelectorAll("div[data-criterion-id]").length === 1, "second step explicitly deleted");
    (await waitFor(() => enabled("exercise-save-draft"), "delete result ready to save")).click();
    await waitFor(() => document.body.innerText.includes("Draft saved. Save configuration when the checklist and references are ready."), "delete result saved");
    return "ok\nDelete and Cancel stay attached to the intended checklist step";
};
