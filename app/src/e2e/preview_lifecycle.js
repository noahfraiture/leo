// Run the production reader against controlled DOM objects without opening a connection.
const runPreviewReaderLifecycle = async (program) => {
    const AsyncFunction = (async () => {}).constructor;
    const execute = new AsyncFunction("window", "document", "dioxus", "Date", "setTimeout", "MediaMTXWebRTCReader", program);
    const original = { isConnected: true, srcObject: null, dataset: { previewOwner: "original" } };
    const replacementStream = {};
    const replacement = { isConnected: true, srcObject: replacementStream, dataset: { previewOwner: "replacement" } };
    let mountedVideo = original;
    let wake;
    let notifySleep;
    let receives = 0;
    let readersCreated = 0;
    const waiting = new Promise((resolve) => { notifySleep = resolve; });
    const fixtureWindow = {};
    const readerConstructor = function (options) { return new fixtureWindow.MediaMTXWebRTCReader(options); };
    const running = execute(fixtureWindow, { getElementById: () => mountedVideo }, {
        recv: async () => receives++ === 0 ? { video_id: "fixture", whep_url: "unused", mount_id: "original" } : undefined,
        send: () => {},
    }, { now: () => 0 }, (callback) => { wake = callback; notifySleep(); }, readerConstructor);
    await Promise.race([
        waiting,
        running.then(() => { throw new Error("Preview reader finished before its loading wait"); }),
    ]);
    original.isConnected = false;
    mountedVideo = replacement;
    fixtureWindow.MediaMTXWebRTCReader = class {
        constructor() { readersCreated++; }
        close() {}
    };
    wake();
    await running;
    if (replacement.srcObject !== replacementStream || readersCreated !== 0) {
        throw new Error("An unmounted preview reader touched the replacement view or opened a connection");
    }

    // Configuration itself can arrive after the replacement is already mounted.
    receives = 0;
    await execute(fixtureWindow, { getElementById: () => replacement }, {
        recv: async () => receives++ === 0 ? { video_id: "fixture", whep_url: "unused", mount_id: "original" } : undefined,
        send: () => {},
    }, { now: () => 0 }, () => { throw new Error("A stale mount unexpectedly waited"); }, readerConstructor);
    if (replacement.srcObject !== replacementStream || readersCreated !== 0) {
        throw new Error("A stale preview configuration claimed the replacement view");
    }

    // A still-mounted view must attach its track and close only its own reader and element.
    const mounted = { isConnected: true, srcObject: null, dataset: { previewOwner: "mounted" } };
    let closed = false;
    let trackAttached = false;
    receives = 0;
    fixtureWindow.MediaMTXWebRTCReader = class {
        constructor(options) {
            if (options.user !== "leo-preview" || options.pass !== "fixture-preview-password") {
                throw new Error("Preview reader did not forward its bridge access password");
            }
            options.onTrack({ streams: [replacementStream] });
            trackAttached = mounted.srcObject === replacementStream;
        }
        close() { closed = true; }
    };
    await execute(fixtureWindow, { getElementById: () => mounted }, {
        recv: async () => receives++ === 0 ? { video_id: "fixture", whep_url: "unused", mount_id: "mounted", access_password: "fixture-preview-password" } : undefined,
        send: () => {},
    }, { now: () => 0 }, () => { throw new Error("A loaded reader unexpectedly waited"); }, readerConstructor);
    if (!trackAttached || !closed || mounted.srcObject !== null) {
        throw new Error("A mounted preview did not attach and clean up its own stream");
    }
};

const verifyPreviewReaderLifecycle = async (program) => {
    let timeout;
    try {
        await Promise.race([
            runPreviewReaderLifecycle(program),
            new Promise((_, reject) => {
                timeout = setTimeout(() => reject(new Error("Preview lifecycle check did not finish")), 5000);
            }),
        ]);
    } finally {
        clearTimeout(timeout);
    }
};
