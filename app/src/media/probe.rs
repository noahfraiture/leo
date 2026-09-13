//! Isolated real-WKWebView playback acceptance, without model calls or production settings.
use backend::analysis::{PlaybackClip, build_evidence, prepare_playback};
use dioxus::prelude::*;
use std::path::PathBuf;

#[derive(Clone)]
struct ProbeInput {
    clip: PlaybackClip,
    result: PathBuf,
}

/// Opens a real desktop video element, verifies decoded playback and a seek, then writes JSON.
pub fn launch_playback_probe(directory: PathBuf, result: PathBuf) {
    let session =
        backend::session::Session::load(&directory.join("events.jsonl")).expect("fixture session");
    let ids = session.cameras.iter().map(|c| c.id).collect::<Vec<_>>();
    let segments = backend::recording::list_segments(&directory.join("recordings"), &ids)
        .expect("fixture segments");
    let evidence = build_evidence(&directory, &session, &segments).expect("fixture evidence");
    let frame = evidence
        .frames
        .iter()
        .find(|f| f.session_offset_ms >= 2000)
        .unwrap_or(&evidence.frames[0]);
    let cache = result
        .parent()
        .expect("result parent")
        .join("playback-cache");
    let clip = prepare_playback(&directory, &evidence, &frame.id, 2000, 8000, &cache)
        .expect("playback conversion");
    dioxus::LaunchBuilder::desktop()
        .with_context(ProbeInput { clip, result })
        .launch(Probe);
}
#[component]
fn Probe() -> Element {
    super::use_media_registry();
    rsx! {
        ProbePlayer {}
    }
}
#[component]
fn ProbePlayer() -> Element {
    let config = use_context::<ProbeInput>();
    let registry = use_context::<super::MediaRegistry>();
    let url = use_hook(|| {
        registry
            .register(&config.clip.path)
            .expect("registered playback")
    });
    let result = config.result.clone();
    let clip = config.clip.clone();
    use_effect(move || {
        let result = result.clone();
        let clip = clip.clone();
        let desktop = dioxus::desktop::window();
        spawn(async move {
            let mut eval = document::eval(
                r#"
                const sleep=ms=>new Promise(r=>setTimeout(r,ms));
                try {
                    let v;for(let i=0;i<100;i++){v=document.getElementById('probe-video');if(v)break;await sleep(50);}
                    if(!v)throw Error('Video element missing');
                    for(let i=0;i<200 && v.readyState<2;i++){if(v.error)throw Error('Decode error '+v.error.code);await sleep(50);}
                    if(v.readyState<2)throw Error('Metadata/decode timeout');
                    v.muted=true;await v.play();await sleep(500);
                    const played=v.currentTime;v.pause();const target=Math.min(v.duration-0.2,v.duration*0.6);v.currentTime=target;
                    for(let i=0;i<100 && (v.seeking || Math.abs(v.currentTime-target)>0.1);i++)await sleep(50);
                    if(played<=0 || v.videoWidth===0 || Math.abs(v.currentTime-target)>0.1)throw Error('Playback or seek did not complete');
                    dioxus.send({ok:true,playedSeconds:played,seekSeconds:v.currentTime,targetSeconds:target,duration:v.duration,width:v.videoWidth,height:v.videoHeight});
                } catch(error) {dioxus.send({ok:false,error:String(error)});}
            "#,
            );
            let outcome = eval
                .recv::<serde_json::Value>()
                .await
                .unwrap_or_else(|e| serde_json::json!({"ok":false,"error":e.to_string()}));
            let report = serde_json::json!({"webview":outcome,"clip":clip});
            let _ = std::fs::write(result, serde_json::to_vec_pretty(&report).unwrap());
            desktop.close();
        });
    });
    rsx! {
        main {
            h1 { "Recording playback check" }
            video {
                id: "probe-video",
                src: url,
                controls: true,
                muted: true,
                playsinline: true,
                style: "width:900px;max-width:100%",
            }
        }
    }
}
