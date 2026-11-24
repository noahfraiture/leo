# WeMM native provider

This loopback server feeds the benchmark's ordered images to Tencent's native WeMM engine. It accepts a single user message containing text and inline JPEG or PNG images. The runner uses greedy decoding and records whether the engine emitted an end token or reached the output limit. Each request starts with a cleared model context. Frame captions retain their position alongside their images.

## Build the pinned runtime

On an Apple Silicon Mac with Python 3.10+, CMake 3.20+, Git and the Xcode command-line tools, run the following in a directory outside the Leo checkout.

```sh
git clone https://github.com/Tencent/WeMM-Video.git
cd WeMM-Video
git checkout --detach 6cefad232b94e3e190d747251f35799b8b142213
CXXFLAGS="-Wno-c++11-narrowing" python3 native/build.py --backend cpu --jobs 6
```

Tencent's build script fetches and verifies llama.cpp revision `88aaf325a7026ac1127d0897394b0a3d6413dc2a`. Its CPU build selects the Metal backend on macOS. The tested build used Apple Clang with `-Wno-c++11-narrowing` on the `wemm-native` target for six list-initialized frame coordinate conversions. The command above passes the same flag at CMake configuration time. Use a fresh build directory so CMake picks up `CXXFLAGS`. The binary used by the adapter is `native/build-cpu/bin/wemm-native`. [Tencent's native runtime](https://github.com/Tencent/WeMM-Video/tree/6cefad232b94e3e190d747251f35799b8b142213/native)

## Start and benchmark

Place `language.gguf` and `mmproj.gguf` from the same official model revision in one directory. The 35B-A3B vision and language files together require 23.45 GiB. Their SHA-256 digests are `79eb545bf0bf8620c58c34f23284934848cdcd791b9b5e0b5f94af0949a65dae` and `98aa3b77e4c8cc128729dc6905259a3723b2dff9dce8555b597343abc2994ec0`, respectively. [Pinned Tencent release](https://huggingface.co/tencent/WeMM-Video-35BA3B-GGUF/tree/94ec458a240bb339fe119c9eae8e9b0c41e3af5e)

From the Leo checkout, substitute the locations of the compiled binary and model files.

```sh
python3 -m benchmarks.providers.wemm \
  --engine /path/to/WeMM-Video/native/build-cpu/bin/wemm-native \
  --weights /path/to/WeMM-Video-35BA3B-GGUF \
  --model tencent/WeMM-Video-35BA3B-GGUF
```

When the server prints its ready address, run the benchmark in another terminal.

```sh
python3 -m benchmarks run --suite perception-confirmation \
  --base-url http://127.0.0.1:18137/v1 \
  --model tencent/WeMM-Video-35BA3B-GGUF \
  --revision 94ec458a240bb339fe119c9eae8e9b0c41e3af5e \
  --max-tokens 2048
```

Defaults are 65,536 context tokens, a batch of 512, six CPU threads, 99 GPU layers and 4 to 768 visual tokens per image. The adapter disables vision flash attention in its child process and uses Tencent's no-thinking generation prefix. Use `--gpu-layers 0` for CPU execution. Stop the server with Ctrl-C to release the model. [Benchmark suites and portable packs](../README.md)

If native inference stalls for 900 seconds, the provider returns HTTP 503 and shuts down its engine. For a longer run on slower hardware, increase both the provider's `--reply-timeout` and the runner's `--timeout`.

## Supplemental native video comparison

Start the same provider with `--paired-video-model wemm-35b-native-pairs` in addition to the main model ID. This exposes a second route on the same loaded engine. Use the separate ID and result directory for the public confirmation suite.

```sh
python3 -m benchmarks run --suite perception-confirmation \
  --base-url http://127.0.0.1:18137/v1 \
  --model wemm-35b-native-pairs \
  --revision 94ec458a240bb339fe119c9eae8e9b0c41e3af5e \
  --max-tokens 2048 --out ~/leo-native-pairs
```

The paired route uses Tencent's `native_prepared_media` interface with `hf_uniform_video_v1`, on the 16 JPEGs already selected for each question. It preserves both timestamps and puts both captions before the adjacent image pair. The visual encoder merges the frames in each pair. The checkpoint records the ordered source hashes, rendered prompt hash, native protocol, and eight visual chunks. The provider requires the expected first-frame IDs for the chunks and checks the number of video-token audit placeholders reported by the engine. The original footage and audio are outside this sampled-frame comparison. [Tencent native inference](https://github.com/Tencent/WeMM-Video/blob/6cefad232b94e3e190d747251f35799b8b142213/native/src/native_runner.cpp) [Pinned frame-grouping implementation](https://github.com/ggml-org/llama.cpp/blob/88aaf325a7026ac1127d0897394b0a3d6413dc2a/tools/mtmd/mtmd.cpp)
