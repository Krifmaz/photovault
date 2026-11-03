//! Running `ffprobe` and `ffmpeg` for streamable video generation.
//!
//! This mirrors `desktop/src/main/services/ffmpeg-worker.ts` in the Ente
//! monorepo (`ffmpegGenerateHLSPlaylistAndSegments`): the same characteristic
//! detection (H.264, HDR, video stream bitrate), the same decisions (re-encode,
//! rescale, tonemap) and the same `ffmpeg` argument list, so that the HLS
//! stream we upload is what the Ente desktop app would have uploaded.
//!
//! The desktop app does not bundle `ffprobe` and instead parses the stream line
//! that `ffmpeg -i` prints. We use `ffprobe -print_format json`, mapping each
//! field to exactly what the desktop app reads from that line:
//!
//! | desktop (stderr stream line)              | here (ffprobe JSON)                    |
//! |-------------------------------------------|----------------------------------------|
//! | line starts with `h264 `                  | `codec_name == "h264"`                 |
//! | contains `smpte2084` / `arib-std-b67`     | `color_transfer` is one of those       |
//! | ` N kb/s` on the video stream line        | `floor(bit_rate / 1000) * 1000`        |
//!
//! Processes are spawned with argument vectors (never through a shell), with
//! stdin closed, and are killed when the caller cancels.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Deserialize;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use super::VideoError;

/// Video bitrate (bits/s) at or below which an H.264 video is copied rather
/// than re-encoded (desktop: `isH264 && bitrate <= 4000 * 1000`).
pub const COPY_MAX_BITRATE: u64 = 4_000 * 1000;

/// Video bitrate (bits/s) at or below which the video is not rescaled to 720p
/// (desktop: `bitrate <= 2000 * 1000`).
pub const NO_RESCALE_MAX_BITRATE: u64 = 2_000 * 1000;

/// Colour transfer characteristics treated as HDR. This is the desktop app's
/// allow-list: false positives make `zscale` fail, false negatives only skip
/// tonemapping.
pub const HDR_TRANSFERS: [&str; 2] = ["smpte2084", "arib-std-b67"];

/// The 720p scale filter used by both the desktop and mobile apps. It scales
/// the shorter side down to at most 720 pixels, keeping the other side even.
pub const SCALE_FILTER: &str =
    "scale='if(lt(iw,ih),min(720,iw),-2)':'if(lt(iw,ih),-2,min(720,ih))'";

/// The HDR → BT.709 tonemapping filters used by the desktop and mobile apps.
pub const TONEMAP_FILTERS: [&str; 3] = [
    "zscale=transfer=linear",
    "tonemap=tonemap=hable:desat=0",
    "zscale=primaries=709:transfer=709:matrix=709",
];

/// Where to find the `ffmpeg` and `ffprobe` executables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegConfig {
    /// Path (or bare name looked up in `PATH`) of `ffmpeg`.
    pub ffmpeg: PathBuf,
    /// Path (or bare name looked up in `PATH`) of `ffprobe`.
    pub ffprobe: PathBuf,
}

impl Default for FfmpegConfig {
    fn default() -> Self {
        Self {
            ffmpeg: PathBuf::from("ffmpeg"),
            ffprobe: PathBuf::from("ffprobe"),
        }
    }
}

/// What `ffprobe` tells us about a video file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VideoProbe {
    /// Container duration in seconds, if known.
    pub duration_secs: Option<f64>,
    /// Coded width of the first video stream.
    pub width: Option<u32>,
    /// Coded height of the first video stream.
    pub height: Option<u32>,
    /// Display rotation in degrees (from the display matrix or `rotate` tag),
    /// 0 when absent.
    pub rotation: i32,
    /// Video stream bitrate in bits/s, rounded down to whole kb/s exactly as
    /// the desktop app reads it from `ffmpeg`'s ` N kb/s`. `None` when the
    /// container does not record a per-stream bitrate (for example WebM).
    pub video_bitrate: Option<u64>,
    /// Container (overall) bitrate in bits/s, informational only.
    pub format_bitrate: Option<u64>,
    /// Codec of the first video stream (for example `h264`, `hevc`).
    pub codec_name: Option<String>,
    /// Colour transfer of the first video stream (for example `smpte2084`).
    pub color_transfer: Option<String>,
    /// Whether the file has a video stream at all.
    pub has_video: bool,
    /// Whether the file has an audio stream.
    pub has_audio: bool,
}

impl VideoProbe {
    /// Whether the video stream is H.264 (desktop: stream line starts `h264 `).
    pub fn is_h264(&self) -> bool {
        self.codec_name.as_deref() == Some("h264")
    }

    /// Whether the video uses an HDR transfer from [`HDR_TRANSFERS`].
    pub fn is_hdr(&self) -> bool {
        self.color_transfer
            .as_deref()
            .is_some_and(|t| HDR_TRANSFERS.contains(&t))
    }

    /// Display dimensions after applying [`rotation`](Self::rotation).
    pub fn display_dimensions(&self) -> Option<(u32, u32)> {
        let (w, h) = (self.width?, self.height?);
        if self.rotation.rem_euclid(180) == 90 {
            Some((h, w))
        } else {
            Some((w, h))
        }
    }

    /// Parse `ffprobe -print_format json -show_format -show_streams` output.
    pub fn from_ffprobe_json(json: &[u8]) -> Result<Self, VideoError> {
        let out: ProbeOutput =
            serde_json::from_slice(json).map_err(|e| VideoError::Probe(e.to_string()))?;
        let mut probe = VideoProbe {
            duration_secs: out.format.as_ref().and_then(|f| parse_num(&f.duration)),
            format_bitrate: out
                .format
                .as_ref()
                .and_then(|f| parse_num(&f.bit_rate))
                .map(|b: f64| b as u64),
            ..Default::default()
        };
        probe.has_audio = out
            .streams
            .iter()
            .any(|s| s.codec_type.as_deref() == Some("audio"));
        // Like the desktop regex, use the first video stream.
        if let Some(v) = out
            .streams
            .iter()
            .find(|s| s.codec_type.as_deref() == Some("video"))
        {
            probe.has_video = true;
            probe.width = v.width.filter(|w| *w > 0);
            probe.height = v.height.filter(|h| *h > 0);
            probe.codec_name = v.codec_name.clone();
            probe.color_transfer = v.color_transfer.clone();
            probe.video_bitrate = parse_num(&v.bit_rate)
                .map(|b: f64| (b as u64) / 1000 * 1000)
                .filter(|b| *b > 0);
            let from_matrix = v
                .side_data_list
                .iter()
                .find_map(|sd| sd.rotation.as_ref().and_then(json_num));
            let from_tag = v
                .tags
                .as_ref()
                .and_then(|t| t.rotate.as_deref())
                .and_then(|r| r.parse::<f64>().ok());
            probe.rotation = from_matrix.or(from_tag).unwrap_or(0.0).round() as i32;
        }
        Ok(probe)
    }
}

fn parse_num(s: &Option<String>) -> Option<f64> {
    s.as_deref()?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

fn json_num(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[derive(Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    bit_rate: Option<String>,
    color_transfer: Option<String>,
    #[serde(default)]
    side_data_list: Vec<SideData>,
    tags: Option<StreamTags>,
}

#[derive(Deserialize)]
struct SideData {
    rotation: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct StreamTags {
    rotate: Option<String>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
    bit_rate: Option<String>,
}

/// The desktop app's per-video choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HlsDecisions {
    /// Re-encode with libx264 (otherwise `-c:v copy`).
    pub reencode: bool,
    /// Scale to at most 720p and 30 fps.
    pub rescale: bool,
    /// Tonemap HDR to BT.709.
    pub tonemap: bool,
}

impl HlsDecisions {
    /// Decide exactly as `ffmpegGenerateHLSPlaylistAndSegments` does.
    pub fn for_probe(probe: &VideoProbe) -> Self {
        let bitrate = probe.video_bitrate;
        Self {
            // Re-encoding is by far the dominant cost of generating the stream.
            reencode: !(probe.is_h264() && bitrate.is_some_and(|b| b <= COPY_MAX_BITRATE)),
            // Avoid shrinking video whose bitrate is already modest.
            rescale: !bitrate.is_some_and(|b| b <= NO_RESCALE_MAX_BITRATE),
            // HDR needs BT.709 tonemapping; applying it to SDR reduces brightness.
            tonemap: probe.is_hdr(),
        }
    }

    /// The `-vf` filter chain, or `None` when the video is copied.
    pub fn video_filter(&self) -> Option<String> {
        if !self.reencode {
            return None;
        }
        let mut filters: Vec<&str> = Vec::new();
        // Tonemapping requires dimensions divisible by chroma subsampling, so
        // it also forces the even-size scale.
        if self.rescale || self.tonemap {
            filters.extend([SCALE_FILTER, "fps=30"]);
        }
        if self.tonemap {
            filters.extend(TONEMAP_FILTERS);
        }
        filters.push("format=yuv420p");
        Some(filters.join(","))
    }
}

/// The `ffmpeg` arguments (excluding the program itself) of the desktop app's
/// HLS command:
///
/// ```text
/// -hide_banner -i INPUT [-vf FILTERS] (-c:v libx264 -maxrate 2000k -bufsize 4000k | -c:v copy)
/// -c:a aac -f hls -hls_key_info_file KEYINFO -hls_list_size 0 -hls_flags single_file PLAYLIST
/// ```
pub fn hls_args(
    input: &Path,
    decisions: &HlsDecisions,
    key_info_path: &Path,
    playlist_path: &Path,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["-hide_banner".into(), "-i".into(), input.into()];
    if let Some(vf) = decisions.video_filter() {
        args.extend(["-vf".into(), vf.into()]);
    }
    if decisions.reencode {
        args.extend(
            ["-c:v", "libx264", "-maxrate", "2000k", "-bufsize", "4000k"].map(OsString::from),
        );
    } else {
        args.extend(["-c:v", "copy"].map(OsString::from));
    }
    args.extend(["-c:a", "aac", "-f", "hls", "-hls_key_info_file"].map(OsString::from));
    args.push(key_info_path.into());
    args.extend(["-hls_list_size", "0", "-hls_flags", "single_file"].map(OsString::from));
    args.push(playlist_path.into());
    args
}

/// The `ffprobe` arguments used by [`probe`].
pub fn probe_args(input: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
    ]
    .map(OsString::from)
    .to_vec();
    args.push(input.into());
    args
}

/// Run `ffprobe` on `input`.
pub async fn probe(
    cfg: &FfmpegConfig,
    input: &Path,
    cancel: &CancellationToken,
) -> Result<VideoProbe, VideoError> {
    let mut cmd = Command::new(&cfg.ffprobe);
    cmd.args(probe_args(input))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| VideoError::Spawn {
        program: cfg.ffprobe.clone(),
        source: e,
    })?;
    let output = tokio::select! {
        out = child.wait_with_output() => out?,
        // Dropping the future drops the child, which kills it (kill_on_drop).
        _ = cancel.cancelled() => return Err(VideoError::Cancelled),
    };
    if !output.status.success() {
        return Err(VideoError::Probe(format!(
            "ffprobe exited with {}: {}",
            output.status,
            tail(&String::from_utf8_lossy(&output.stderr))
        )));
    }
    VideoProbe::from_ffprobe_json(&output.stdout)
}

/// Run `ffmpeg` with `args`, sending its stderr to `stderr_path` (as the
/// desktop app does, to avoid unbounded buffering on long videos).
///
/// When `cancel` fires the child is killed and [`VideoError::Cancelled`] is
/// returned.
pub async fn run_ffmpeg(
    cfg: &FfmpegConfig,
    args: &[OsString],
    stderr_path: &Path,
    cancel: &CancellationToken,
) -> Result<(), VideoError> {
    if cancel.is_cancelled() {
        return Err(VideoError::Cancelled);
    }
    let stderr = std::fs::File::create(stderr_path)?;
    let mut child = Command::new(&cfg.ffmpeg)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr))
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| VideoError::Spawn {
            program: cfg.ffmpeg.clone(),
            source: e,
        })?;
    let status = tokio::select! {
        st = child.wait() => st?,
        _ = cancel.cancelled() => {
            // Best effort: the process may have exited already.
            let _ = child.kill().await;
            return Err(VideoError::Cancelled);
        }
    };
    if !status.success() {
        let text = tokio::fs::read_to_string(stderr_path)
            .await
            .unwrap_or_default();
        return Err(VideoError::Ffmpeg {
            status: status.to_string(),
            stderr_tail: tail(&text),
        });
    }
    Ok(())
}

/// The output video dimensions from an `ffmpeg` conversion's stderr.
///
/// Like the desktop app's `detectVideoDimensions`, this takes the last
/// `Stream #…: Video:` line (output streams are listed after input streams)
/// and the first ` WxH` in it whose numbers have no leading zero (so that
/// hexadecimal codec tags such as `0x31637661` do not match).
pub fn parse_output_dimensions(stderr: &str) -> Option<(u32, u32)> {
    let line = stderr
        .lines()
        .filter(|l| l.contains("Stream #"))
        // The desktop regex's greedy `.+` before `: Video:` takes the last one.
        .filter_map(|l| l.rsplit_once(": Video:").map(|(_, rest)| rest))
        .next_back()?;
    line.match_indices(' ')
        .find_map(|(i, _)| dims_at(&line[i + 1..]))
}

/// Match `([1-9]\d*)x([1-9]\d*)` at the start of `s`.
fn dims_at(s: &str) -> Option<(u32, u32)> {
    let (w, rest) = leading_nonzero_int(s)?;
    let rest = rest.strip_prefix('x')?;
    let (h, _) = leading_nonzero_int(rest)?;
    Some((w, h))
}

fn leading_nonzero_int(s: &str) -> Option<(u32, &str)> {
    if !s.starts_with(|c: char| ('1'..='9').contains(&c)) {
        return None;
    }
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    Some((s[..end].parse().ok()?, &s[end..]))
}

/// The last few lines of a process's stderr, for error messages.
fn tail(text: &str) -> String {
    const LINES: usize = 24;
    const CHARS: usize = 4000;
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(LINES);
    let joined = lines[start..].join("\n");
    let skip = joined.chars().count().saturating_sub(CHARS);
    joined.chars().skip(skip).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    /// Fixture: the desktop command (`ffmpeg-worker.ts` at b2ed3dc), flattened
    /// for each combination of decisions, transcribed by hand from the source.
    fn desktop_fixture(reencode: bool, rescale: bool, tonemap: bool) -> Vec<&'static str> {
        let mut v = vec!["-hide_banner", "-i", "INPUT"];
        if reencode {
            v.push("-vf");
            v.push(match (rescale || tonemap, tonemap) {
                (false, _) => "format=yuv420p",
                (true, false) => "scale='if(lt(iw,ih),min(720,iw),-2)':'if(lt(iw,ih),-2,min(720,ih))',fps=30,format=yuv420p",
                (true, true) => "scale='if(lt(iw,ih),min(720,iw),-2)':'if(lt(iw,ih),-2,min(720,ih))',fps=30,zscale=transfer=linear,tonemap=tonemap=hable:desat=0,zscale=primaries=709:transfer=709:matrix=709,format=yuv420p",
            });
            v.extend(["-c:v", "libx264", "-maxrate", "2000k", "-bufsize", "4000k"]);
        } else {
            v.extend(["-c:v", "copy"]);
        }
        v.extend([
            "-c:a",
            "aac",
            "-f",
            "hls",
            "-hls_key_info_file",
            "KEYINFO",
            "-hls_list_size",
            "0",
            "-hls_flags",
            "single_file",
            "PLAYLIST",
        ]);
        v
    }

    #[test]
    fn args_match_desktop_for_every_decision() {
        for reencode in [false, true] {
            for rescale in [false, true] {
                for tonemap in [false, true] {
                    let d = HlsDecisions {
                        reencode,
                        rescale,
                        tonemap,
                    };
                    let got = strs(&hls_args(
                        Path::new("INPUT"),
                        &d,
                        Path::new("KEYINFO"),
                        Path::new("PLAYLIST"),
                    ));
                    assert_eq!(got, desktop_fixture(reencode, rescale, tonemap), "{d:?}");
                }
            }
        }
    }

    fn probe_with(codec: &str, bitrate: Option<u64>, transfer: Option<&str>) -> VideoProbe {
        VideoProbe {
            codec_name: Some(codec.into()),
            video_bitrate: bitrate,
            color_transfer: transfer.map(Into::into),
            has_video: true,
            ..Default::default()
        }
    }

    #[test]
    fn decisions_match_desktop() {
        let d = HlsDecisions::for_probe(&probe_with("h264", Some(1_500_000), None));
        assert_eq!(
            d,
            HlsDecisions {
                reencode: false,
                rescale: false,
                tonemap: false
            }
        );
        let d = HlsDecisions::for_probe(&probe_with("h264", Some(4_000_000), None));
        assert!(!d.reencode && d.rescale);
        let d = HlsDecisions::for_probe(&probe_with("h264", Some(4_001_000), None));
        assert!(d.reencode && d.rescale);
        // Unknown bitrate: re-encode and rescale.
        let d = HlsDecisions::for_probe(&probe_with("h264", None, None));
        assert!(d.reencode && d.rescale);
        // HEVC is always re-encoded; low bitrate skips rescale.
        let d = HlsDecisions::for_probe(&probe_with("hevc", Some(1_000_000), None));
        assert!(d.reencode && !d.rescale && !d.tonemap);
        let d = HlsDecisions::for_probe(&probe_with("hevc", Some(1_000_000), Some("arib-std-b67")));
        assert!(d.tonemap);
        assert!(d.video_filter().unwrap().starts_with(SCALE_FILTER));
        // bt709 and other transfers are not HDR.
        assert!(!probe_with("hevc", None, Some("bt709")).is_hdr());
        assert!(probe_with("hevc", None, Some("smpte2084")).is_hdr());
    }

    #[test]
    fn parses_ffprobe_json() {
        let json = br#"{"streams":[
            {"codec_type":"audio","codec_name":"aac","bit_rate":"69000"},
            {"codec_type":"video","codec_name":"hevc","width":1920,"height":1080,
             "bit_rate":"52266","color_transfer":"arib-std-b67",
             "side_data_list":[{"side_data_type":"Display Matrix","rotation":-90}]}],
            "format":{"duration":"3.000000","bit_rate":"130000"}}"#;
        let p = VideoProbe::from_ffprobe_json(json).unwrap();
        assert!(p.has_video && p.has_audio);
        assert_eq!(p.video_bitrate, Some(52_000));
        assert_eq!(p.format_bitrate, Some(130_000));
        assert_eq!(p.duration_secs, Some(3.0));
        assert_eq!(p.rotation, -90);
        assert_eq!(p.display_dimensions(), Some((1080, 1920)));
        assert!(p.is_hdr() && !p.is_h264());
    }

    #[test]
    fn parses_output_dimensions_like_desktop() {
        let stderr = "\
  Stream #0:0[0x1](und): Video: h264 (High 4:4:4 Predictive) (avc1 / 0x31637661), yuv444p(progressive), 640x360 [SAR 1:1 DAR 16:9], 52 kb/s, 30 fps
Stream mapping:
  Stream #0:0 -> #0:0 (h264 (native) -> h264 (libx264))
  Stream #0:0(und): Video: h264, yuv420p(tv, progressive), 404x720 [SAR 1:1 DAR 101:180], q=2-31, 30 fps, 90k tbn (default)
  Stream #0:1(und): Audio: aac (LC), 44100 Hz, mono, fltp, 69 kb/s (default)
";
        assert_eq!(parse_output_dimensions(stderr), Some((404, 720)));
        assert_eq!(
            parse_output_dimensions("  Stream #0:0: Video: h264 (avc1 / 0x31637661), 0x0"),
            None
        );
        assert_eq!(parse_output_dimensions("nothing"), None);
    }
}
