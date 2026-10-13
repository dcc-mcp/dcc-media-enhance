use crate::{
    contract::{self, Error, Job, Result},
    process,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Media {
    pub format_name: String,
    pub width: u64,
    pub height: u64,
    pub pixel_format: String,
    pub video_streams: usize,
    pub audio_codecs: Vec<String>,
    pub audio_extradata_sha256: Vec<Option<String>>,
    pub color_transfer: String,
    pub frame_rate: String,
    pub duration: Option<f64>,
    pub frames: Option<u64>,
    pub probe_level: String,
    pub sample_aspect_ratio: String,
    pub rotation_degrees: f64,
    pub unsupported_streams: usize,
}
// Source identity covers one retained file only. Never admit playlist/sequence
// demuxers or network protocols whose referenced inputs evade that identity.
const INPUT_FORMATS: &str = "png_pipe,jpeg_pipe,webp_pipe,ppm_pipe,pgm_pipe,bmp_pipe,tiff_pipe,exr_pipe,mov,matroska,webm,avi,mpegts,mpeg,ogg";
fn file_input_options() -> Vec<String> {
    [
        "-protocol_whitelist",
        "file",
        "-format_whitelist",
        INPUT_FORMATS,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}
fn probe_input_options() -> Vec<String> {
    let mut args = file_input_options();
    // ffprobe accepts demuxer options across formats. ffmpeg only accepts these
    // for MOV, where execute() selects them using the actually probed format.
    args.extend(
        ["-enable_drefs", "0", "-use_absolute_path", "0"]
            .iter()
            .map(|s| s.to_string()),
    );
    args
}
fn media(path: &Path, count: bool) -> Result<Media> {
    media_with_cancel(path, count, None)
}
fn media_with_cancel(path: &Path, count: bool, cancel: Option<&Path>) -> Result<Media> {
    process::check_cancel(cancel)?;
    contract::regular_file(path, u64::MAX)?;
    // FFprobe cannot decode Blender's multilayer named-channel files. Do not
    // silently accept its 0x0 stream. The initial EXR probe is explicitly header-only.
    if extension(path) == "exr" && !count {
        return exr_header(path);
    }
    let exe = process::executable("ffprobe")?;
    let mut args = vec![
        "-v".into(),
        "error".into(),
        "-threads".into(),
        "1".into(),
        "-show_streams".into(),
        "-show_data_hash".into(),
        "sha256".into(),
        "-show_format".into(),
        "-of".into(),
        "json".into(),
    ];
    if count {
        args.push("-count_frames".into());
    }
    args.extend(probe_input_options());
    args.push(path.to_string_lossy().into_owned());
    let d: Value = serde_json::from_str(&process::capture_with_cancel(&exe, &args, cancel)?)?;
    let streams = d["streams"]
        .as_array()
        .ok_or_else(|| Error::new("INVALID_MEDIA", "No readable media streams", 2))?;
    let videos: Vec<&Value> = streams
        .iter()
        .filter(|s| s["codec_type"] == "video")
        .collect();
    let v = videos
        .first()
        .ok_or_else(|| Error::new("INVALID_MEDIA", "No video or image stream", 2))?;
    Ok(Media {
        format_name: d["format"]["format_name"].as_str().unwrap_or("").into(),
        width: v["width"].as_u64().unwrap_or(0),
        height: v["height"].as_u64().unwrap_or(0),
        pixel_format: v["pix_fmt"].as_str().unwrap_or("").into(),
        video_streams: videos.len(),
        audio_codecs: streams
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .map(|s| s["codec_name"].as_str().unwrap_or("unknown").into())
            .collect(),
        audio_extradata_sha256: streams
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .map(|s| s["extradata_hash"].as_str().map(str::to_owned))
            .collect(),
        color_transfer: v["color_transfer"].as_str().unwrap_or("unspecified").into(),
        frame_rate: v["r_frame_rate"].as_str().unwrap_or("").into(),
        duration: d["format"]["duration"]
            .as_str()
            .and_then(|s| s.parse().ok()),
        frames: v[if count { "nb_read_frames" } else { "nb_frames" }]
            .as_str()
            .and_then(|s| s.parse().ok()),
        sample_aspect_ratio: v["sample_aspect_ratio"].as_str().unwrap_or("1:1").into(),
        rotation_degrees: v["side_data_list"]
            .as_array()
            .and_then(|a| a.iter().find_map(|x| x["rotation"].as_f64()))
            .or_else(|| v["tags"]["rotate"].as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(0.0),
        unsupported_streams: streams
            .iter()
            .filter(|s| s["codec_type"] != "video" && s["codec_type"] != "audio")
            .count(),
        probe_level: if count {
            "decoded_frame_count"
        } else {
            "stream_metadata"
        }
        .into(),
    })
}

fn exr_header(path: &Path) -> Result<Media> {
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(65536).read_to_end(&mut bytes)?;
    let invalid = || {
        Error::new(
            "INVALID_EXR",
            "Requires a bounded flat single-part EXR v2 header with zero-offset matching data/display windows",
            2,
        )
    };
    if bytes.len() < 9 || bytes[..4] != [0x76, 0x2f, 0x31, 0x01] {
        return Err(invalid());
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if version & 255 != 2 || version & (0x800 | 0x1000) != 0 {
        return Err(invalid());
    }
    fn zstr<'a>(bytes: &'a [u8], pos: &mut usize) -> Option<&'a [u8]> {
        let n = bytes.get(*pos..)?.iter().position(|b| *b == 0)?;
        if n > 255 {
            return None;
        }
        let s = &bytes[*pos..*pos + n];
        *pos += n + 1;
        Some(s)
    }
    let mut pos = 8;
    let mut data = None;
    let mut display = None;
    let mut pixel_aspect = 1.0f32;
    let mut ended = false;
    while pos < bytes.len() {
        let name = zstr(&bytes, &mut pos).ok_or_else(invalid)?;
        if name.is_empty() {
            ended = true;
            break;
        }
        let kind = zstr(&bytes, &mut pos).ok_or_else(invalid)?;
        let lenbytes = bytes.get(pos..pos + 4).ok_or_else(invalid)?;
        let n = u32::from_le_bytes(lenbytes.try_into().unwrap()) as usize;
        pos += 4;
        let value = bytes
            .get(pos..pos.checked_add(n).ok_or_else(invalid)?)
            .ok_or_else(invalid)?;
        pos += n;
        if name == b"pixelAspectRatio" {
            if kind != b"float" || n != 4 {
                return Err(invalid());
            }
            pixel_aspect = f32::from_le_bytes(value.try_into().unwrap());
        }
        if name == b"dataWindow" || name == b"displayWindow" {
            if kind != b"box2i" || n != 16 {
                return Err(invalid());
            }
            let mut bounds = [0i32; 4];
            for (i, b) in bounds.iter_mut().enumerate() {
                *b = i32::from_le_bytes(value[i * 4..i * 4 + 4].try_into().unwrap());
            }
            if name == b"dataWindow" {
                data = Some(bounds)
            } else {
                display = Some(bounds)
            }
        }
    }
    let d = data.ok_or_else(invalid)?;
    if pixel_aspect != 1.0
        || !ended
        || display != Some(d)
        || d[0] != 0
        || d[1] != 0
        || d[2] < 0
        || d[3] < 0
    {
        return Err(invalid());
    }
    Ok(Media {
        format_name: "exr".into(),
        width: d[2] as u64 + 1,
        height: d[3] as u64 + 1,
        pixel_format: "exr_channels_not_decoded".into(),
        video_streams: 1,
        audio_codecs: vec![],
        audio_extradata_sha256: vec![],
        color_transfer: "scene-linear_declared_by_caller".into(),
        frame_rate: "".into(),
        duration: None,
        frames: None,
        probe_level: "header_only".into(),
        sample_aspect_ratio: "1:1".into(),
        rotation_degrees: 0.0,
        unsupported_streams: 0,
    })
}
pub fn probe(path: &Path) -> Result<Value> {
    Ok(
        json!({"schema_version":1,"success":true,"media":media(path,false)?,"sha256":contract::hash_file(path)?}),
    )
}

pub fn doctor() -> Result<Value> {
    let mut backends = Vec::new();
    for name in [
        "ffmpeg",
        "ffprobe",
        "blender",
        "oidnDenoise",
        "waifu2x-ncnn-vulkan",
    ] {
        match process::executable(name) {
            Ok(p) => {
                let version = if name == "ffmpeg" || name == "ffprobe" {
                    process::capture(&p, &["-version".into()])
                } else if name == "blender" {
                    process::capture(&p, &["--version".into()])
                } else {
                    Ok("present; backend version/capabilities not yet verified".into())
                };
                backends.push(json!({"executable":name,"path":p,"present":true,"version":version.ok().map(|s|s.lines().take(2).collect::<Vec<_>>().join("\n")),"sha256":contract::hash_file(&p)?}));
            }
            Err(_) => backends.push(json!({"executable":name,"present":false})),
        }
    }
    let filters = process::executable("ffmpeg")
        .and_then(|p| process::capture(&p, &["-hide_banner".into(), "-filters".into()]))
        .unwrap_or_default();
    Ok(
        json!({"schema_version":1,"success":true,"version":env!("CARGO_PKG_VERSION"),"platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,
        "capacity_observations":{"mem_available_kib":fs::read_to_string("/proc/meminfo").ok().and_then(|s|s.lines().find(|l|l.starts_with("MemAvailable:")).and_then(|l|l.split_whitespace().nth(1)).and_then(|n|n.parse::<u64>().ok())),"cgroup_memory_max":fs::read_to_string("/sys/fs/cgroup/memory.max").ok().map(|s|s.trim().to_owned()),"cgroup_cpu_max":fs::read_to_string("/sys/fs/cgroup/cpu.max").ok().map(|s|s.trim().to_owned()),"free_disk_bytes":process::disk_free(&std::env::current_dir()?).ok(),"note":"Observations are not reservations; concurrent workloads and parent cgroups may constrain capacity."},"logical_cpus":std::thread::available_parallelism().map(|n|n.get()).unwrap_or(1),"backends":backends,
        "ffmpeg_filters":{"nlmeans":filters.lines().any(|l|l.split_whitespace().nth(1)==Some("nlmeans")),"atadenoise":filters.lines().any(|l|l.split_whitespace().nth(1)==Some("atadenoise"))},
        "default_device":"cpu","memory_enforcement":process::memory_enforcement(),"network_required_for_inference":false,
        "supported_platforms":["linux","windows","macos"],"cache_reuse_supported":process::cache_reuse_supported(),"process_namespace":process::pid_namespace(),"supervisor_identity":process::process_identity(std::process::id()),"abrupt_supervisor_death_cleanup":cfg!(target_os="linux") || cfg!(windows),"cluster_scheduler_included":false}),
    )
}

#[derive(Debug, Serialize)]
pub struct Plan {
    pub schema_version: u32,
    pub success: bool,
    pub input: PathBuf,
    pub output: PathBuf,
    pub source_sha256: String,
    pub backend: String,
    pub executable: PathBuf,
    pub executable_sha256: String,
    pub runtime_fingerprint: Value,
    pub limits: contract::Limits,
    pub media: Media,
    pub cache_key: String,
    pub model_hashes: Value,
    pub warnings: Vec<String>,
    pub expected_width: u64,
    pub expected_height: u64,
}
fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase()
}
pub fn plan(job: &Job, base: &Path) -> Result<Plan> {
    job.validate()?;
    crate::platform::validate_path(&job.input)?;
    crate::platform::validate_path(&job.output)?;
    let original = contract::resolve(base, &job.input);
    contract::regular_file(&original, job.limits.max_input_mb * 1024 * 1024)?;
    let input = fs::canonicalize(original)?;
    let dest = contract::resolve(base, &job.output);
    let parent = fs::canonicalize(
        dest.parent()
            .ok_or_else(|| Error::new("INVALID_PATH", "Output needs a parent directory", 2))?,
    )?;
    let output = parent.join(
        dest.file_name()
            .ok_or_else(|| Error::new("INVALID_PATH", "Output needs a filename", 2))?,
    );
    if input == output {
        return Err(Error::new(
            "SOURCE_OVERWRITE",
            "In-place enhancement is never supported",
            2,
        ));
    }
    if fs::symlink_metadata(&output).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(Error::new("UNSAFE_OUTPUT", "Output cannot be a symlink", 2));
    }
    if process::disk_free(&parent)? < job.limits.min_free_disk_mb * 1024 * 1024 {
        return Err(Error::new(
            "DISK_LIMIT",
            "Insufficient free disk for requested reserve",
            4,
        ));
    }
    let source_sha256 = contract::hash_file(&input)?;
    let render_input =
        job.media == "image" && job.content == "render" && extension(&input) == "exr";
    let mut m = media(&input, false)?;
    if m.video_streams != 1
        || m.width == 0
        || m.height == 0
        || m.width
            .checked_mul(m.height)
            .is_none_or(|p| p > job.limits.max_pixels)
    {
        return Err(Error::new(
            "MEDIA_LIMIT",
            "Requires one bounded video/image stream",
            2,
        ));
    }
    if job.media == "image" && !render_input {
        m = media(&input, true)?;
    }
    if job.media == "image" && !render_input && (!m.audio_codecs.is_empty() || m.frames != Some(1))
    {
        return Err(Error::new(
            "MEDIA_MISMATCH",
            "An image job must decode to exactly one frame without audio",
            2,
        ));
    }
    if m.rotation_degrees != 0.0
        || !["1:1", "0:1", "N/A"].contains(&m.sample_aspect_ratio.as_str())
        || m.unsupported_streams != 0
    {
        return Err(Error::new(
            "UNSUPPORTED_LAYOUT",
            "MVP requires square pixels, no rotation metadata, and only one video stream plus optional audio",
            2,
        ));
    }
    let backend = if job.backend == "auto" {
        match (
            job.operation.as_str(),
            job.media.as_str(),
            job.content.as_str(),
        ) {
            ("denoise", "image", "render") => "blender-oidn",
            ("denoise", "image", _) => "ffmpeg-nlmeans",
            ("denoise", "video", _) => "ffmpeg-atadenoise",
            ("upscale", "image", _) => "waifu2x-cpu",
            ("resize", "image", _) => "ffmpeg-lanczos",
            _ => {
                return Err(Error::new(
                    "UNSUPPORTED_OPERATION",
                    "No validated backend route for this combination",
                    2,
                ));
            }
        }
        .to_owned()
    } else {
        job.backend.clone()
    };
    let mut warnings = vec![
        "Enhanced output is a derivative; source is retained unchanged.".into(),
        format!("Memory policy: {}.", process::memory_enforcement()),
    ];
    if !process::cache_reuse_supported() {
        warnings.push("Full native loader identity is not verified on this platform; existing outputs are not reused as cache entries. Choose a new output for reruns.".into());
    }
    if cfg!(target_os="macos") {
        warnings.push("macOS: cooperative cancellation and owned process-group cleanup are supported; abrupt supervisor SIGKILL cannot guarantee descendant cleanup. Use an external job supervisor for that guarantee.".into());
    }
    let (name, ext) = match backend.as_str() {
        "blender-oidn"
            if job.operation == "denoise" && job.media == "image" && job.content == "render" =>
        {
            if extension(&input) != "exr"
                || job.colorspace != "scene-linear"
                || job.passes.is_none()
            {
                return Err(Error::new(
                    "RENDER_CONTRACT",
                    "Render backend requires scene-linear EXR and explicit real beauty/albedo/normal/layer mappings",
                    2,
                ));
            }
            warnings.push("Output EXR contains denoised RGBA beauty only. All original passes remain in the immutable source EXR.".into());
            ("blender", "exr")
        }
        "ffmpeg-nlmeans"
            if job.operation == "denoise" && job.media == "image" && job.content != "render" =>
        {
            ("ffmpeg", "png")
        }
        "ffmpeg-atadenoise" if job.operation == "denoise" && job.media == "video" => {
            warnings.push("Temporal averaging may cause ghosting; no motion-compensated or neural temporal guarantee.".into());
            warnings.push("Video is lossless FFV1/Matroska; audio packets are copied. Timestamp and audio-payload checks are required before commit; time origin is normalized.".into());
            ("ffmpeg", "mkv")
        }
        "ffmpeg-lanczos" if job.operation == "resize" && job.media == "image" => ("ffmpeg", "png"),
        "waifu2x-cpu"
            if job.operation == "upscale" && job.media == "image" && job.content != "render" =>
        {
            warnings.push("Experimental adapter: an accepted plan does not prove CPU inference works on this machine. Require a local inference smoke test before production use.".into());
            warnings.push("Neural upscale can synthesize details. Never use the derivative as proof of unreadable text or scene facts.".into());
            ("waifu2x-ncnn-vulkan", "png")
        }
        _ => {
            return Err(Error::new(
                "UNSUPPORTED_BACKEND",
                "Backend does not implement this operation/media/content contract",
                2,
            ));
        }
    };
    if extension(&output) != ext {
        return Err(Error::new(
            "OUTPUT_FORMAT",
            format!("This backend requires .{ext} output"),
            2,
        ));
    }
    if backend != "blender-oidn" {
        if job.colorspace != "srgb"
            || ![
                "rgb24", "bgr24", "gbrp", "gray", "yuv420p", "yuv422p", "yuv444p", "yuvj420p",
                "yuvj422p", "yuvj444p",
            ]
            .contains(&m.pixel_format.as_str())
            || ["smpte2084", "arib-std-b67"].contains(&m.color_transfer.as_str())
        {
            return Err(Error::new(
                "UNSUPPORTED_COLOR",
                "MVP non-render backends require opaque 8-bit SDR sRGB/BT.709 media; no implicit HDR, alpha or high-bit-depth conversion",
                2,
            ));
        }
        warnings.push("The caller declares the source SDR color interpretation. ICC/EXIF preservation is not implemented; the original retains its metadata.".into());
    }
    let scale = if job.operation == "denoise" {
        if job.scale.is_some() {
            return Err(Error::new("INVALID_SCALE", "Denoise must not resize", 2));
        }
        1
    } else {
        let s = job.scale.unwrap_or(2);
        if s != 2 {
            return Err(Error::new(
                "INVALID_SCALE",
                "MVP only supports explicit 2x scaling",
                2,
            ));
        }
        s
    };
    let ew = m.width * scale as u64;
    let eh = m.height * scale as u64;
    if ew.checked_mul(eh).is_none_or(|n| n > job.limits.max_pixels) {
        return Err(Error::new("MEDIA_LIMIT", "Output pixels exceed budget", 2));
    }
    let mut models = serde_json::Map::new();
    if backend == "waifu2x-cpu" {
        let dir=contract::resolve(base,job.model_dir.as_ref().ok_or_else(||Error::new("MODEL_REQUIRED","Provide an audited local waifu2x model_dir; no model is downloaded automatically",2))?);
        let mut paths = fs::read_dir(&dir)?
            .map(|e| e.map(|x| x.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        paths.sort();
        if paths.len() > 100 {
            return Err(Error::new(
                "MODEL_LIMIT",
                "Model directory has more than 100 entries",
                2,
            ));
        }
        for p in paths {
            if ["bin", "param"].contains(&extension(&p).as_str()) {
                contract::regular_file(&p, 128 * 1024 * 1024)?;
                models.insert(
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    json!(contract::hash_file(&p)?),
                );
            }
        }
        if models.is_empty() {
            return Err(Error::new(
                "MODEL_REQUIRED",
                "No ncnn .bin/.param files in model directory",
                2,
            ));
        }
    } else if job.model_dir.is_some() {
        return Err(Error::new(
            "UNEXPECTED_MODEL",
            "Only waifu2x-cpu accepts model_dir",
            2,
        ));
    }
    if backend == "waifu2x-cpu"
        && !["jpg", "jpeg", "png", "webp"].contains(&extension(&input).as_str())
    {
        return Err(Error::new(
            "UNSUPPORTED_INPUT",
            "waifu2x CLI accepts jpg/png/webp inputs; provide an explicit conversion first",
            2,
        ));
    }
    let executable = process::executable(name)?;
    let executable_sha256 = contract::hash_file(&executable)?;
    if name == "ffmpeg" && backend != "ffmpeg-lanczos" {
        let f = if backend == "ffmpeg-nlmeans" {
            "nlmeans"
        } else {
            "atadenoise"
        };
        let filters = process::capture(&executable, &["-hide_banner".into(), "-filters".into()])?;
        if !filters
            .lines()
            .any(|l| l.split_whitespace().nth(1) == Some(f))
        {
            return Err(Error::new(
                "CAPABILITY_MISSING",
                format!("Installed FFmpeg does not expose {f}"),
                3,
            ));
        }
    }
    let runtime_fingerprint = process::runtime_fingerprint(&executable)?;
    let cache_key = contract::hash_bytes(&serde_json::to_vec(
        &json!({"job":job,"resolved_input":input,"resolved_output":output,"source":source_sha256,"backend":backend,"binary":executable_sha256,"runtime":runtime_fingerprint,"models":models,"helper":contract::hash_bytes(include_bytes!("../backend/blender_denoise.py")),"engine":contract::hash_bytes(include_bytes!("engine.rs")),"runner":contract::hash_bytes(include_bytes!("process.rs")),"portable_runner":contract::hash_bytes(include_bytes!("process_portable.rs")),"platform":contract::hash_bytes(include_bytes!("platform.rs")),"windows":contract::hash_bytes(include_bytes!("windows.rs")),"contract":contract::hash_bytes(include_bytes!("contract.rs")),"cli":contract::hash_bytes(include_bytes!("main.rs")),"version":env!("CARGO_PKG_VERSION")}),
    )?);
    Ok(Plan {
        schema_version: 1,
        success: true,
        input,
        output,
        source_sha256,
        backend,
        executable,
        executable_sha256,
        runtime_fingerprint,
        limits: job.limits.clone(),
        media: m,
        cache_key,
        model_hashes: Value::Object(models),
        warnings,
        expected_width: ew,
        expected_height: eh,
    })
}

struct Lock {
    _file: fs::File,
}
fn acquire_lock(path: &Path, owner: &str) -> Result<Lock> {
    let (mut file, new) = match crate::platform::lock_file(path, true) {
        Ok(f) => (f, true),
        Err(e) if e.code == "IO_ERROR" && contract::path_exists(path)? =>
            (crate::platform::lock_file(path, false)?, false),
        Err(e) => return Err(e),
    };
    let md = file.metadata()?;
    if !md.is_file() || md.len() > 4096 {
        return Err(Error::new(
            "LOCK_NOT_OWNED",
            "Lock must be a bounded regular file",
            3,
        ));
    }
    crate::platform::lock(&file)?;
    if !new {
        let mut text = String::new();
        Read::by_ref(&mut file)
            .take(4096)
            .read_to_string(&mut text)?;
        let v: Value = serde_json::from_str(&text).map_err(|_| {
            Error::new(
                "LOCK_NOT_OWNED",
                "Existing lock file is not a recognized tool sidecar",
                3,
            )
        })?;
        if v["tool"] != "dcc-media-enhance" {
            return Err(Error::new(
                "LOCK_NOT_OWNED",
                "Do not overwrite a foreign lock file",
                3,
            ));
        }
    }
    file.rewind()?;
    file.set_len(0)?;
    file.write_all(
        serde_json::to_string(&json!({"tool":"dcc-media-enhance","job_id":owner}))?.as_bytes(),
    )?;
    file.sync_all()?;
    Ok(Lock { _file: file })
}
struct CancelCleanup {
    path: PathBuf,
    id: String,
}
impl Drop for CancelCleanup {
    fn drop(&mut self) {
        if contract::regular_file(&self.path, 256).is_ok()
            && fs::read_to_string(&self.path).ok().as_deref() == Some(&self.id)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn complete_cached_manifest(m: &Value, p: &Plan, job: &Job) -> bool {
    let Ok(out) = serde_json::from_value::<Media>(m["output_media"].clone()) else {
        return false;
    };
    let receipt = &m["resource_receipt"];
    let video = &m["verification"]["video"];
    let video_valid = if job.media == "video" {
        [
            "frame_count_equal",
            "audio_codec_sequence_equal",
            "relative_frame_timestamps_equal",
            "audio_packet_sha256_equal",
            "audio_extradata_equal",
            "audio_packet_side_data_equal",
            "audio_relative_timestamps_equal",
        ]
        .iter()
        .all(|k| video[k] == true)
            && video["timestamp_tolerance_seconds"] == 0.0011
            && video["absolute_origin"] == "normalized"
    } else {
        video.is_null() && out.frames == Some(1)
    };
    process::cache_reuse_supported()
        && m["schema_version"] == 1
        && m["success"] == true
        && m["status"] == "completed"
        && m["cache_key"] == p.cache_key
        && m["cache_hit"].is_boolean()
        && m["job_id"]
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 128)
        && m["input"] == json!(p.input)
        && m["output"] == json!(p.output)
        && m["request"] == json!(job)
        && m["source_sha256"] == p.source_sha256
        && m["source_unchanged"] == true
        && m["backend"]["id"] == p.backend
        && m["backend"]["binary"] == json!(p.executable)
        && m["backend"]["sha256"] == p.executable_sha256
        && m["backend"]["runtime"] == p.runtime_fingerprint
        && m["backend"]["model_hashes"] == p.model_hashes
        && m["warnings"] == json!(p.warnings)
        && m["cache_scope"].is_string()
        && receipt["elapsed_ms"].as_u64().is_some()
        && receipt["sampled_peak_rss_kib"].as_u64().is_some()
        && receipt["exit_code"] == 0
        && receipt["memory_enforcement"] == process::memory_enforcement()
        && out.width == p.expected_width
        && out.height == p.expected_height
        && out.frames.is_some_and(|n| n > 0)
        && out.video_streams == 1
        && m["verification"]["pixel_decode"] == true
        && m["verification"]["dimensions"] == true
        && m["verification"]["quality_reference"] == "not_run"
        && m["verification"]["visual_review"] == "not_run"
        && video_valid
        && m["provenance"]["kind"] == "enhanced_derivative"
        && m["provenance"]["factual_detail_restoration_guaranteed"] == false
}

pub fn run(job: &Job, base: &Path) -> Result<Value> {
    let p = plan(job, base)?;
    let manifest = contract::suffix(&p.output, ".manifest.json");
    if contract::path_exists(&p.output)? {
        if let Ok(m) = contract::read_json_bounded(&manifest, 1_048_576) {
            if complete_cached_manifest(&m, &p, job)
                && m["output_sha256"] == contract::hash_file(&p.output)?
            {
                let mut hit = m;
                hit["cache_hit"] = json!(true);
                return Ok(hit);
            }
        }
        return Err(Error::new(
            "OUTPUT_EXISTS",
            "Existing output is not a verified matching cache entry; choose a new output path",
            2,
        ));
    }
    if contract::path_exists(&manifest)? {
        return Err(Error::new(
            "MANIFEST_EXISTS",
            "An output manifest already exists; do not overwrite it",
            2,
        ));
    }
    let id = format!("{}-{}", contract::now_ms(), std::process::id());
    let lock_path = contract::suffix(&p.output, ".lock-owner");
    let _lock = acquire_lock(&lock_path, &id)?;
    let work = p.output.parent().unwrap().join(format!(".dcc-media-{id}"));
    fs::create_dir(&work)?;
    crate::platform::private_directory(&work)?;
    let staging = work.join(format!("result.{}", extension(&p.output)));
    let log = work.join("backend.log");
    let state = contract::suffix(&p.output, ".job.json");
    let cancel = state.with_extension("cancel");
    if contract::path_exists(&state)? {
        let previous = contract::read_json_bounded(&state, 1_048_576)?;
        if previous["tool"] != "dcc-media-enhance" {
            return Err(Error::new(
                "STATE_NOT_OWNED",
                "Existing job sidecar is not owned by this tool",
                3,
            ));
        }
        if contract::path_exists(&cancel)?
            && contract::regular_file(&cancel, 256).is_ok()
            && fs::read_to_string(&cancel).ok().as_deref() == previous["job_id"].as_str()
        {
            fs::remove_file(&cancel)?;
        }
    }
    if contract::path_exists(&cancel)? {
        return Err(Error::new(
            "STALE_CANCEL",
            "Unrecognized cancellation sentinel; do not overwrite it",
            2,
        ));
    }
    let _cancel_cleanup = CancelCleanup {
        path: cancel.clone(),
        id: id.clone(),
    };
    contract::write_json(
        &state,
        &json!({"tool":"dcc-media-enhance","schema_version":1,"job_id":id,"status":"running","cache_key":p.cache_key,"state_path":state,"output":p.output,"cancel_path":cancel,"started_ms":contract::now_ms(),"supervisor_pid":std::process::id(),"supervisor_identity":process::process_identity(std::process::id()),"supervisor_pid_namespace":process::pid_namespace()}),
    )?;
    let outcome = execute(job, base, &p, &work, &staging, &log, &cancel);
    match outcome {
        Ok(mut result) => {
            result["job_id"] = json!(id);
            result["cache_key"] = json!(p.cache_key);
            result["status"] = json!("completed");
            result["cache_hit"] = json!(false);
            process::check_cancel(Some(&cancel)).map_err(|e|{let _=fs::remove_file(&staging);let _=contract::write_json(&state,&json!({"tool":"dcc-media-enhance","schema_version":1,"job_id":id,"status":"cancelled"}));e})?;
            contract::write_json(
                &state,
                &json!({"tool":"dcc-media-enhance","schema_version":1,"job_id":id,"status":"committing","output":p.output,"manifest":manifest,"supervisor_pid":std::process::id(),"supervisor_identity":process::process_identity(std::process::id()),"supervisor_pid_namespace":process::pid_namespace()}),
            )?;
            // The manifest is the commit marker. A filesystem cannot atomically
            // publish two independent names; a crash may leave a complete orphan
            // output, which is never automatically trusted or overwritten.
            let published = (|| -> Result<()> {
                fs::OpenOptions::new().write(true).open(&staging)?.sync_all()?;
                let mf = work.join("manifest.json");
                contract::write_json(&mf, &result)?;
                process::check_cancel(Some(&cancel))?;
                fs::hard_link(&staging, &p.output)
                    .map_err(|e| Error::new("PUBLICATION_CONFLICT", e.to_string(), 3))?;
                contract::sync_parent(&p.output)?;
                fs::hard_link(&mf,&manifest).map_err(|e|Error::new("PUBLICATION_INCOMPLETE",format!("Complete output exists without its commit manifest: {e}; inspect the recorded staging directory"),3))?;
                contract::sync_parent(&manifest)?;
                Ok(())
            })();
            if let Err(e) = published {
                if e.code == "CANCELLED" {
                    let _ = fs::remove_file(&staging);
                }
                let _ = contract::write_json(
                    &state,
                    &json!({"tool":"dcc-media-enhance","schema_version":1,"job_id":id,"status":if e.code=="CANCELLED"{"cancelled"}else{"publication_incomplete"},"error":{"code":e.code,"message":e.message},"staging":staging,"manifest_candidate":work.join("manifest.json"),"output":p.output}),
                );
                return Err(e);
            }
            let _ = fs::remove_file(&staging);
            // A durable manifest already proves success even if the advisory state
            // file cannot subsequently be refreshed (for example disk full).
            if contract::write_json(&state,&json!({"tool":"dcc-media-enhance","schema_version":1,"job_id":id,"status":"completed","manifest":manifest,"output":p.output})).is_err(){result["state_update_warning"]=json!("Manifest committed; advisory state refresh failed");}
            Ok(result)
        }
        Err(mut e) => {
            if let Err(cancelled) = process::check_cancel(Some(&cancel)) {
                e = cancelled;
            }
            let _ = fs::remove_file(&staging);
            if let Err(state_error) = contract::write_json(
                &state,
                &json!({"tool":"dcc-media-enhance","schema_version":1,"job_id":id,"status":if e.code=="CANCELLED"{"cancelled"}else{"failed"},"error":{"code":e.code,"message":e.message},"diagnostic_log":log,"source_unchanged":contract::hash_file(&p.input).ok().map(|sha|sha==p.source_sha256)}),
            ) {
                // Preserve the primary cancellation/resource failure. In
                // particular a failed advisory state write must not let a
                // cancelled batch continue to its next job.
                e.message.push_str(&format!(
                    "; advisory state update failed: {}",
                    state_error.message
                ));
            }
            Err(e)
        }
    }
}

fn execute(
    job: &Job,
    base: &Path,
    p: &Plan,
    work: &Path,
    staging: &Path,
    log: &Path,
    cancel: &Path,
) -> Result<Value> {
    let s = |x: &Path| x.to_string_lossy().into_owned();
    let args = if p.backend == "blender-oidn" {
        let script = work.join("blender_denoise.py");
        fs::write(&script, include_bytes!("../backend/blender_denoise.py"))?;
        let config = work.join("backend-job.json");
        contract::write_json(
            &config,
            &json!({"input":p.input,"output":staging,"passes":job.passes,"threads":job.limits.threads,"max_pixels":job.limits.max_pixels}),
        )?;
        vec![
            "--background".into(),
            "--factory-startup".into(),
            "--disable-autoexec".into(),
            "--threads".into(),
            job.limits.threads.to_string(),
            "--python-exit-code".into(),
            "1".into(),
            "--python".into(),
            s(&script),
            "--".into(),
            s(&config),
        ]
    } else if p.backend == "waifu2x-cpu" {
        vec![
            "-i".into(),
            s(&p.input),
            "-o".into(),
            s(staging),
            "-g".into(),
            "-1".into(),
            "-n".into(),
            "-1".into(),
            "-s".into(),
            "2".into(),
            "-t".into(),
            "32".into(),
            "-j".into(),
            format!("1:{}:1", job.limits.threads),
            "-m".into(),
            s(&contract::resolve(base, job.model_dir.as_ref().unwrap())),
        ]
    } else {
        let mut a = vec![
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-nostdin".into(),
            "-n".into(),
            "-threads".into(),
            job.limits.threads.to_string(),
            "-filter_threads".into(),
            job.limits.threads.to_string(),
        ];
        if job.media == "video" {
            a.extend(["-copyts".into(), "-start_at_zero".into()]);
        }
        a.extend(file_input_options());
        if p.media.format_name.split(',').any(|f| f == "mov") {
            a.extend(
                ["-enable_drefs", "0", "-use_absolute_path", "0"]
                    .iter()
                    .map(|s| s.to_string()),
            );
        }
        a.extend(["-i".into(), s(&p.input), "-map".into(), "0:v:0".into()]);
        if job.media == "video" {
            a.extend([
                "-map".into(),
                "0:a?".into(),
                "-c:a".into(),
                "copy".into(),
                "-map_metadata".into(),
                "0".into(),
                "-map_chapters".into(),
                "0".into(),
                "-fps_mode".into(),
                "passthrough".into(),
                "-vf".into(),
                "atadenoise=s=5".into(),
                "-c:v".into(),
                "ffv1".into(),
                "-level".into(),
                "3".into(),
            ]);
        } else {
            let filter = if p.backend == "ffmpeg-nlmeans" {
                "format=gbrp,nlmeans=s=2:p=3:r=7,format=rgb24".into()
            } else {
                format!(
                    "scale={}:{}:flags=lanczos,format=rgb24",
                    p.expected_width, p.expected_height
                )
            };
            a.extend([
                "-vf".into(),
                filter,
                "-frames:v".into(),
                "1".into(),
                "-c:v".into(),
                "png".into(),
            ]);
        }
        a.extend([
            "-threads".into(),
            job.limits.threads.to_string(),
            s(staging),
        ]);
        a
    };
    if contract::hash_file_with_cancel(&p.executable, Some(cancel))? != p.executable_sha256 {
        return Err(Error::new(
            "BACKEND_CHANGED",
            "Backend binary changed since planning",
            3,
        ));
    }
    let receipt = process::run(
        &p.executable,
        &args,
        &job.limits,
        log,
        Some(staging),
        Some(cancel),
    )?;
    contract::regular_file(staging, job.limits.max_output_mb * 1024 * 1024)?;
    let out = media_with_cancel(staging, true, Some(cancel))?;
    if out.width != p.expected_width || out.height != p.expected_height {
        return Err(Error::new(
            "VERIFY_DIMENSIONS",
            "Output dimensions do not match plan",
            3,
        ));
    }
    if job.media == "image" && out.frames != Some(1) {
        return Err(Error::new(
            "VERIFY_FRAMES",
            "Image output did not decode as exactly one frame",
            3,
        ));
    }
    let mut video_verification = json!(null);
    if job.media == "video" {
        let original = media_with_cancel(&p.input, true, Some(cancel))?;
        if original.frames.is_none()
            || original.frames != out.frames
            || original.audio_codecs != out.audio_codecs
            || original.audio_extradata_sha256 != out.audio_extradata_sha256
        {
            return Err(Error::new(
                "VERIFY_STREAMS",
                "Output frame count/audio codecs do not match source",
                3,
            ));
        }
        video_verification =
            verify_video(&p.input, staging, original.audio_codecs.len(), Some(cancel))?;
    }
    // Full decode with error promotion; header checks alone are not sufficient.
    let ffmpeg = process::executable("ffmpeg")?;
    let mut decode_args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-xerror".into(),
        "-nostdin".into(),
        "-threads".into(),
        "1".into(),
    ];
    decode_args.extend(file_input_options());
    decode_args.extend([
        "-i".into(),
        s(staging),
        "-map".into(),
        "0:v:0".into(),
        "-threads".into(),
        "1".into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ]);
    process::run(
        &ffmpeg,
        &decode_args,
        &job.limits,
        &work.join("verify-decode.log"),
        None,
        Some(cancel),
    )?;
    if contract::hash_file_with_cancel(&p.executable, Some(cancel))? != p.executable_sha256 {
        return Err(Error::new(
            "BACKEND_CHANGED",
            "Backend binary changed during processing",
            3,
        ));
    }
    if let Some(libraries) = p.runtime_fingerprint["linked_libraries"].as_object() {
        for (path, sha) in libraries {
            if Some(contract::hash_file_with_cancel(Path::new(path), Some(cancel))?.as_str())
                != sha.as_str()
            {
                return Err(Error::new(
                    "RUNTIME_CHANGED",
                    "Backend runtime changed during processing",
                    3,
                ));
            }
        }
    }
    if let Some(dir) = &job.model_dir {
        let dir = contract::resolve(base, dir);
        for (name, sha) in p.model_hashes.as_object().unwrap() {
            if Some(contract::hash_file_with_cancel(&dir.join(name), Some(cancel))?.as_str())
                != sha.as_str()
            {
                return Err(Error::new(
                    "MODEL_CHANGED",
                    "Model file changed during processing",
                    3,
                ));
            }
        }
    }
    if contract::hash_file_with_cancel(&p.input, Some(cancel))? != p.source_sha256 {
        return Err(Error::new(
            "SOURCE_CHANGED",
            "Source changed during processing; result was not published",
            3,
        ));
    }
    if process::disk_free(p.output.parent().unwrap())? < job.limits.min_free_disk_mb * 1024 * 1024 {
        return Err(Error::new(
            "DISK_LIMIT",
            "Free disk reserve exhausted before publication",
            4,
        ));
    }
    Ok(
        json!({"schema_version":1,"success":true,"request":job,"input":p.input,"output":p.output,"source_sha256":p.source_sha256,"output_sha256":contract::hash_file_with_cancel(staging,Some(cancel))?,"source_unchanged":true,
        "backend":{"id":p.backend,"binary":p.executable,"sha256":p.executable_sha256,"model_hashes":p.model_hashes,"runtime":p.runtime_fingerprint},
        "cache_scope":"binary, linked runtime, model, helper and effective request identities; independently loaded plugins are unsupported","output_media":out,"resource_receipt":receipt,"warnings":p.warnings,
        "verification":{"pixel_decode":true,"dimensions":true,"video":video_verification,"quality_reference":"not_run","visual_review":"not_run"},"provenance":{"kind":"enhanced_derivative","factual_detail_restoration_guaranteed":false}}),
    )
}

fn video_timing(path: &Path, cancel: Option<&Path>) -> Result<Vec<f64>> {
    let ffprobe = process::executable("ffprobe")?;
    let args = [
        "-v",
        "error",
        "-threads",
        "1",
        "-select_streams",
        "v:0",
        "-show_frames",
        "-show_entries",
        "frame=best_effort_timestamp_time",
        "-of",
        "json",
    ]
    .iter()
    .map(|x| x.to_string())
    .chain(probe_input_options())
    .chain([path.to_string_lossy().into_owned()])
    .collect::<Vec<_>>();
    let v: Value = serde_json::from_str(&process::capture_with_cancel(&ffprobe, &args, cancel)?)?;
    v["frames"]
        .as_array()
        .ok_or_else(|| Error::new("VERIFY_TIMING", "No frame timestamps", 3))?
        .iter()
        .map(|f| {
            let n = f["best_effort_timestamp_time"]
                .as_str()
                .and_then(|n| n.parse::<f64>().ok())
                .filter(|n| n.is_finite());
            n.ok_or_else(|| Error::new("VERIFY_TIMING", "Missing or invalid frame timestamp", 3))
        })
        .collect()
}
fn audio_packets(
    path: &Path,
    index: usize,
    cancel: Option<&Path>,
) -> Result<Vec<(f64, String, Value)>> {
    let ffprobe = process::executable("ffprobe")?;
    let args = [
        "-v",
        "error",
        "-threads",
        "1",
        "-select_streams",
        &format!("a:{index}"),
        "-show_packets",
        "-show_data_hash",
        "sha256",
        "-show_entries",
        "packet=pts_time,data_hash:packet_side_data",
        "-of",
        "json",
    ]
    .iter()
    .map(|x| x.to_string())
    .chain(probe_input_options())
    .chain([path.to_string_lossy().into_owned()])
    .collect::<Vec<_>>();
    let v: Value = serde_json::from_str(&process::capture_with_cancel(&ffprobe, &args, cancel)?)?;
    v["packets"]
        .as_array()
        .ok_or_else(|| Error::new("VERIFY_AUDIO", "No audio packet list", 3))?
        .iter()
        .map(|f| {
            let n = f["pts_time"]
                .as_str()
                .and_then(|n| n.parse::<f64>().ok())
                .filter(|n| n.is_finite());
            let h = f["data_hash"]
                .as_str()
                .filter(|h| h.starts_with("SHA256:") && h.len() == 71);
            match (n, h) {
                (Some(n), Some(h)) => Ok((
                    n,
                    h.to_owned(),
                    f["side_data_list"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .into(),
                )),
                _ => Err(Error::new(
                    "VERIFY_AUDIO",
                    "Missing packet hash or timestamp",
                    3,
                )),
            }
        })
        .collect()
}
fn verify_video(
    input: &Path,
    output: &Path,
    audio_count: usize,
    cancel: Option<&Path>,
) -> Result<Value> {
    let a = video_timing(input, cancel)?;
    let b = video_timing(output, cancel)?;
    if a.is_empty() || a.len() != b.len() {
        return Err(Error::new(
            "VERIFY_TIMING",
            "Frame timestamp counts differ",
            3,
        ));
    }
    let tolerance = 0.0011; // Matroska millisecond timebase; no frame-rate conversion.
    if a.iter()
        .zip(&b)
        .any(|(x, y)| ((x - a[0]) - (y - b[0])).abs() > tolerance)
    {
        return Err(Error::new(
            "VERIFY_TIMING",
            "Relative presentation timestamps changed beyond 1.1 ms",
            3,
        ));
    }
    for index in 0..audio_count {
        let x = audio_packets(input, index, cancel)?;
        let y = audio_packets(output, index, cancel)?;
        if x.len() != y.len()
            || x.iter().zip(&y).any(|((xt, xh, xs), (yt, yh, ys))| {
                xh != yh || xs != ys || ((xt - a[0]) - (yt - b[0])).abs() > tolerance
            })
        {
            return Err(Error::new(
                "VERIFY_AUDIO",
                "Audio payload or A/V timing changed",
                3,
            ));
        }
    }
    Ok(
        json!({"frame_count_equal":true,"audio_codec_sequence_equal":true,"relative_frame_timestamps_equal":true,"audio_packet_sha256_equal":true,"audio_extradata_equal":true,"audio_packet_side_data_equal":true,"audio_relative_timestamps_equal":true,"timestamp_tolerance_seconds":tolerance,"absolute_origin":"normalized"}),
    )
}

pub fn batch(path: &Path) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Batch {
        schema_version: u32,
        jobs: Vec<Job>,
    }
    let b: Batch = serde_json::from_value(contract::read_json_bounded(path, 1_048_576)?)?;
    if b.schema_version != 1 || b.jobs.is_empty() || b.jobs.len() > 100 {
        return Err(Error::new(
            "INVALID_BATCH",
            "Batch requires schema 1 and 1–100 jobs",
            2,
        ));
    }
    let base = fs::canonicalize(path)?.parent().unwrap().to_path_buf();
    let mut results = Vec::new();
    let mut success = true;
    let mut cancelled = false;
    for (i, j) in b.jobs.iter().enumerate() {
        match run(j, &base) {
            Ok(r) => results.push(json!({"index":i,"result":r})),
            Err(e) => {
                success = false;
                results.push(json!({"index":i,"error":{"code":e.code,"message":e.message}}));
                if e.code == "CANCELLED" {
                    cancelled = true;
                    break;
                }
            }
        }
    }
    Ok(
        json!({"schema_version":1,"success":success,"cancelled":cancelled,"not_started":b.jobs.len()-results.len(),"execution":"serial","resume":if process::cache_reuse_supported(){"completed items reuse matching verified output manifests"}else{"cache reuse disabled on this platform; skip completed outputs explicitly or choose new output paths"},"results":results}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lock_excludes_other_supervisor_and_recovers_after_drop() {
        let dir = std::env::temp_dir().join(format!(
            "dcc-lock-test-{}-{}",
            std::process::id(),
            contract::now_ms()
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("owned.lock");
        let first = acquire_lock(&path, "first").unwrap();
        assert!(matches!(acquire_lock(&path,"second"),Err(e) if e.code=="OUTPUT_LOCKED"));
        drop(first);
        let recovered = acquire_lock(&path, "recovered").unwrap();
        drop(recovered);
        fs::write(dir.join("foreign.lock"), "not ours").unwrap();
        assert!(
            matches!(acquire_lock(&dir.join("foreign.lock"),"third"),Err(e) if e.code=="LOCK_NOT_OWNED")
        );
        assert_eq!(
            fs::read_to_string(dir.join("foreign.lock")).unwrap(),
            "not ours"
        );
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn malformed_exr_header_is_rejected() {
        let path = std::env::temp_dir().join(format!(
            "dcc-exr-test-{}-{}.exr",
            std::process::id(),
            contract::now_ms()
        ));
        fs::write(&path, b"not an exr").unwrap();
        assert!(matches!(exr_header(&path),Err(e) if e.code=="INVALID_EXR"));
        fs::remove_file(path).unwrap();
    }
}
