//! End-to-end checks of streamable video generation against what the Ente
//! apps expect.
//!
//! Each test makes a short video with `ffmpeg -f lavfi` (testsrc + sine), runs
//! [`photovault_node::video::generate`], and then plays the result back the way
//! an Ente client would: the playlist template must pass the web app's
//! `reconstructHLSPlaylist` rules, every byte-range segment must decrypt with
//! the AES-128 key and IV from the `#EXT-X-KEY` line, and the decrypted
//! MPEG-TS must decode with `ffmpeg` at the advertised dimensions. The
//! reconstructed playlist (pointing at the encrypted object) is also played by
//! `ffmpeg`'s own HLS demuxer, which is what the apps' players do.
//!
//! The tests are skipped (with a message) when `ffmpeg`/`ffprobe` are missing.

use std::path::{Path, PathBuf};
use std::process::Command;

use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use photovault_node::video::{
    self, FfmpegConfig, GenerateOutcome, HlsOutput, PlaylistJson, UploadPlan, VideoError,
};
use tokio_util::sync::CancellationToken;

fn have_ffmpeg() -> bool {
    let ok = |p: &str| {
        Command::new(p)
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    let have = ok("ffmpeg") && ok("ffprobe");
    if !have {
        eprintln!("skipping: ffmpeg/ffprobe not installed");
    }
    have
}

/// Run `ffmpeg` with lavfi inputs to make a test clip.
fn make_clip(out: &Path, size: &str, secs: u32, video_args: &[&str]) {
    let mut args: Vec<String> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.push(format!("testsrc=size={size}:rate=30:duration={secs}"));
    args.extend(["-f", "lavfi", "-i"].map(String::from));
    args.push(format!("sine=frequency=440:duration={secs}"));
    args.extend(video_args.iter().map(|s| s.to_string()));
    args.extend(["-c:a", "aac", "-shortest"].map(String::from));
    args.push(out.to_str().unwrap().to_owned());
    let st = Command::new("ffmpeg").args(&args).status().unwrap();
    assert!(st.success(), "making test clip failed: {args:?}");
}

/// A parsed playlist template: key, IV and byte ranges.
struct Parsed {
    key: [u8; 16],
    iv: [u8; 16],
    ranges: Vec<(u64, u64)>,
    duration: f64,
}

fn parse_template(t: &str) -> Parsed {
    let mut key = None;
    let mut iv = None;
    let mut ranges = Vec::new();
    let mut duration = 0.0;
    for line in t.lines() {
        if let Some(k) =
            line.strip_prefix("#EXT-X-KEY:METHOD=AES-128,URI=\"data:text/plain;base64,")
        {
            let (b64, ivhex) = k.split_once("\",IV=0x").unwrap();
            key = Some(B64.decode(b64).unwrap().try_into().unwrap());
            iv = Some(hex::decode(ivhex).unwrap().try_into().unwrap());
        } else if let Some(br) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            let (n, o) = br.split_once('@').unwrap();
            ranges.push((o.parse().unwrap(), n.parse().unwrap()));
        } else if let Some(d) = line.strip_prefix("#EXTINF:") {
            duration += d.trim_end_matches(',').parse::<f64>().unwrap();
        }
    }
    Parsed {
        key: key.expect("key"),
        iv: iv.expect("iv"),
        ranges,
        duration,
    }
}

/// Decrypt every segment like an HLS player and return the plaintext TS.
fn decrypt_segments(out: &HlsOutput) -> Vec<u8> {
    let p = parse_template(&out.playlist_template);
    let data = std::fs::read(&out.encrypted_video_path).unwrap();
    assert_eq!(data.len() as u64, out.size);
    // The segments tile the object exactly.
    let mut next = 0;
    let mut plain = Vec::new();
    for (off, len) in &p.ranges {
        assert_eq!(*off, next);
        next = off + len;
        let seg = &data[*off as usize..next as usize];
        let dec = cbc::Decryptor::<aes::Aes128>::new(&p.key.into(), &p.iv.into())
            .decrypt_padded_vec_mut::<Pkcs7>(seg)
            .expect("segment decrypts with the playlist key");
        assert_eq!(dec[0], 0x47, "decrypted segment is MPEG-TS");
        plain.extend(dec);
    }
    assert_eq!(next, out.size);
    plain
}

/// First video stream's fields from `ffprobe -of json`.
fn ffprobe_video(path: &Path, entries: &str) -> serde_json::Value {
    let o = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .arg(format!("stream={entries}"))
        .args(["-of", "json"])
        .arg(path)
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    v["streams"][0].clone()
}

fn ffprobe_dims(path: &Path) -> (u32, u32) {
    let v = ffprobe_video(path, "width,height");
    (
        v["width"].as_u64().unwrap() as u32,
        v["height"].as_u64().unwrap() as u32,
    )
}

fn ffprobe_codec(path: &Path) -> String {
    ffprobe_video(path, "codec_name")["codec_name"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The desktop app's `detectVideoCharacteristics`: run its pseudo-ffprobe
/// command (`ffmpeg -hide_banner -i IN -an -frames:v 0 -f null -`) and read
/// the first `Stream #…: Video:` line exactly as `ffmpeg-worker.ts` does.
/// Returns `(isH264, isHDR, bitrate)`.
fn desktop_characteristics(input: &Path) -> (bool, bool, Option<u64>) {
    let o = Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-i")
        .arg(input)
        .args(["-an", "-frames:v", "0", "-f", "null", "-"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&o.stderr);
    let Some(line) = stderr
        .lines()
        .filter(|l| l.contains("Stream #"))
        .find_map(|l| l.rsplit_once(": Video:").map(|(_, r)| r.trim()))
    else {
        return (false, false, None);
    };
    let is_h264 = line.starts_with("h264 ");
    let is_hdr = line.contains("smpte2084") || line.contains("arib-std-b67");
    // / ([1-9]\d*) kb\/s/
    let bitrate = line.split(", ").find_map(|part| {
        let n = part.strip_suffix(" kb/s")?.rsplit(' ').next()?;
        (!n.starts_with('0')).then(|| n.parse::<u64>().ok().map(|k| k * 1000))?
    });
    (is_h264, is_hdr, bitrate)
}

/// Our ffprobe-based characteristics agree with the desktop app's parse.
async fn assert_matches_desktop(input: &Path) -> video::VideoProbe {
    let probe = video::ffmpeg::probe(&FfmpegConfig::default(), input, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(
        (probe.is_h264(), probe.is_hdr(), probe.video_bitrate),
        desktop_characteristics(input),
        "{probe:?}"
    );
    probe
}

fn decodes(args: &[&str], input: &Path) {
    let o = Command::new("ffmpeg")
        .args(["-hide_banner", "-v", "error", "-xerror"])
        .args(args)
        .arg("-i")
        .arg(input)
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "decode of {input:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

/// Generate a stream for `input` and verify it as a client would.
async fn generate_and_verify(input: &Path, dir: &Path) -> (HlsOutput, PathBuf) {
    let work = dir.join("work");
    let out = match video::generate(
        &FfmpegConfig::default(),
        input,
        &work,
        &CancellationToken::new(),
    )
    .await
    .unwrap()
    {
        GenerateOutcome::Generated(o) => o,
        GenerateOutcome::NotRequired => panic!("expected a stream"),
    };

    // Only the playlist and the encrypted object remain; the key is gone.
    let mut left: Vec<String> = std::fs::read_dir(&work)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left, ["output.m3u8", "output.ts"]);

    // The template passes the web app's rules.
    video::validate_playlist_template(&out.playlist_template).unwrap();

    // The object is not plaintext MPEG-TS.
    let raw = std::fs::read(&out.encrypted_video_path).unwrap();
    assert!(raw.len() < 188 || raw[0] != 0x47 || raw[188] != 0x47);

    // Decrypt each segment and decode the result.
    let plain = dir.join("decrypted.ts");
    std::fs::write(&plain, decrypt_segments(&out)).unwrap();
    assert_eq!(ffprobe_dims(&plain), (out.width, out.height));
    decodes(&[], &plain);

    // Play the reconstructed playlist (segments → the encrypted object) with
    // ffmpeg's HLS demuxer, like the apps' players.
    let url = out.encrypted_video_path.to_str().unwrap();
    let playlist = video::reconstruct_playlist(&out.playlist_template, url).unwrap();
    let m3u8 = dir.join("play.m3u8");
    std::fs::write(&m3u8, playlist).unwrap();
    decodes(
        &[
            "-allowed_extensions",
            "ALL",
            "-protocol_whitelist",
            "file,data,crypto",
        ],
        &m3u8,
    );
    (out, plain)
}

#[tokio::test]
async fn landscape_reencode_and_rescale() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.mp4");
    // MPEG-4 part 2 at a high bitrate: not H.264, so always re-encoded, and
    // rescaled to 720p because the bitrate is over 2 Mbit/s.
    make_clip(&input, "1920x1080", 3, &["-c:v", "mpeg4", "-b:v", "8M"]);
    assert_matches_desktop(&input).await;
    let (out, plain) = generate_and_verify(&input, dir.path()).await;
    assert_eq!((out.width, out.height), (1280, 720));
    assert_eq!(ffprobe_codec(&plain), "h264");
    let p = parse_template(&out.playlist_template);
    assert!((p.duration - 3.0).abs() < 0.2, "duration {}", p.duration);
}

#[tokio::test]
async fn portrait_low_bitrate_keeps_size() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.mp4");
    // Low bitrate: re-encoded (not H.264) but not rescaled.
    make_clip(&input, "360x640", 3, &["-c:v", "mpeg4", "-b:v", "500k"]);
    let (out, _) = generate_and_verify(&input, dir.path()).await;
    assert_eq!((out.width, out.height), (360, 640));
}

#[tokio::test]
async fn portrait_high_bitrate_h264_rescales_short_side() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.mp4");
    // H.264 over 10 MiB and over 4 Mbit/s: re-encoded and rescaled so the
    // short side is 720.
    make_clip(
        &input,
        "1080x1920",
        4,
        &[
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-b:v",
            "40M",
            "-minrate",
            "40M",
            "-maxrate",
            "40M",
            "-bufsize",
            "40M",
            "-x264-params",
            "nal-hrd=cbr",
        ],
    );
    assert!(std::fs::metadata(&input).unwrap().len() > video::H264_SKIP_MAX_SIZE);
    assert_matches_desktop(&input).await;
    let (out, _) = generate_and_verify(&input, dir.path()).await;
    assert_eq!((out.width, out.height), (720, 1280));
}

#[tokio::test]
async fn h264_moderate_bitrate_is_copied() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.mp4");
    // H.264 over 10 MiB at ~3 Mbit/s: copied, not re-encoded.
    make_clip(
        &input,
        "640x360",
        32,
        &[
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-b:v",
            "3M",
            "-minrate",
            "3M",
            "-maxrate",
            "3M",
            "-bufsize",
            "6M",
            "-x264-params",
            "nal-hrd=cbr",
        ],
    );
    let size = std::fs::metadata(&input).unwrap().len();
    assert!(size > video::H264_SKIP_MAX_SIZE, "clip is {size} bytes");
    let probe = video::ffmpeg::probe(&FfmpegConfig::default(), &input, &CancellationToken::new())
        .await
        .unwrap();
    assert_matches_desktop(&input).await;
    let d = video::HlsDecisions::for_probe(&probe);
    assert!(!d.reencode, "{probe:?}");
    let (out, _) = generate_and_verify(&input, dir.path()).await;
    assert_eq!((out.width, out.height), (640, 360));
}

#[tokio::test]
async fn hdr_is_tonemapped() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.mp4");
    make_clip(
        &input,
        "640x360",
        2,
        &[
            "-c:v",
            "libx265",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p10le",
            "-color_primaries",
            "bt2020",
            "-color_trc",
            "smpte2084",
            "-colorspace",
            "bt2020nc",
            "-x265-params",
            "log-level=error",
        ],
    );
    let probe = video::ffmpeg::probe(&FfmpegConfig::default(), &input, &CancellationToken::new())
        .await
        .unwrap();
    assert!(probe.is_hdr(), "{probe:?}");
    assert_matches_desktop(&input).await;
    assert!(video::HlsDecisions::for_probe(&probe).tonemap);
    let (out, plain) = generate_and_verify(&input, dir.path()).await;
    assert_eq!((out.width, out.height), (640, 360));
    let v = ffprobe_video(&plain, "pix_fmt,color_transfer");
    assert_eq!(v["pix_fmt"], "yuv420p", "{v}");
    assert_ne!(v["color_transfer"], "smpte2084", "{v}");
}

#[tokio::test]
async fn small_h264_is_not_required() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.mp4");
    make_clip(
        &input,
        "320x240",
        2,
        &["-c:v", "libx264", "-preset", "ultrafast"],
    );
    let r = video::generate(
        &FfmpegConfig::default(),
        &input,
        &dir.path().join("work"),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(r, GenerateOutcome::NotRequired);
}

#[tokio::test]
async fn cancellation_kills_ffmpeg() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.mp4");
    make_clip(&input, "1920x1080", 20, &["-c:v", "mpeg4", "-q:v", "2"]);
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        c2.cancel();
    });
    let work = dir.path().join("work");
    let started = std::time::Instant::now();
    let r = video::generate(&FfmpegConfig::default(), &input, &work, &cancel).await;
    assert!(matches!(r, Err(VideoError::Cancelled)), "{r:?}");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    // The one-time key does not outlive a cancelled run.
    assert!(!work.join("output.m3u8.key").exists());
    assert!(!work.join("output.ts").exists());
}

#[tokio::test]
async fn missing_ffmpeg_is_a_spawn_error() {
    let cfg = FfmpegConfig {
        ffmpeg: "/nonexistent/ffmpeg".into(),
        ffprobe: "/nonexistent/ffprobe".into(),
    };
    let r = video::generate(
        &cfg,
        Path::new("/nonexistent/in.mp4"),
        Path::new("/nonexistent/work"),
        &CancellationToken::new(),
    )
    .await;
    assert!(matches!(r, Err(VideoError::Spawn { .. })), "{r:?}");
}

/// The full upload shape: plan, playlist JSON encrypted with the file key,
/// and the `PUT /files/video-data` body, decrypted back as the web app does.
#[tokio::test]
async fn video_data_request_round_trips() {
    if !have_ffmpeg() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("in.mp4");
    make_clip(&input, "640x360", 2, &["-c:v", "mpeg4"]);
    let (out, _) = generate_and_verify(&input, dir.path()).await;

    let plan = UploadPlan::for_file(&out).await.unwrap();
    assert!(!plan.is_multipart());
    let bytes = plan.read_part(&plan.parts[0]).await.unwrap();
    assert_eq!(bytes, std::fs::read(&out.encrypted_video_path).unwrap());

    photovault_node::crypto::init().unwrap();
    let key = photovault_node::crypto::SecretKey(rand::random());
    let req = video::video_data_request(42, "pv_0123456789abcdefABCDEF", &out, &key).unwrap();
    let body = serde_json::to_value(&req).unwrap();
    assert_eq!(body["fileID"], 42);
    assert_eq!(body["objectSize"], out.size);
    let blob = photovault_node::crypto::EncryptedBlob {
        encrypted_data: B64.decode(&req.playlist).unwrap(),
        decryption_header: B64.decode(&req.playlist_header).unwrap(),
    };
    let pj = PlaylistJson::decrypt(&blob, &key).unwrap();
    assert_eq!(pj.kind, "hls_video");
    assert_eq!(pj.playlist, out.playlist_template);
    assert_eq!(
        (pj.width, pj.height, pj.size),
        (out.width, out.height, out.size)
    );
}
