//! The parsers against real files, in whole and in awkward chunkings.

use ferrite_mse::{Codec, Container, Event, Parser, Sample, TrackInfo, TrackKind};
use std::collections::BTreeMap;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

struct Parsed {
    tracks: Vec<TrackInfo>,
    samples: BTreeMap<u32, Vec<Sample>>,
    inits: usize,
}

fn parse(container: Container, data: &[u8], chunk: usize) -> Parsed {
    let mut p = Parser::new(container);
    let mut out = Parsed {
        tracks: Vec::new(),
        samples: BTreeMap::new(),
        inits: 0,
    };
    for piece in data.chunks(chunk) {
        for ev in p.append(piece).expect("parse") {
            match ev {
                Event::Init { tracks, .. } => {
                    out.inits += 1;
                    out.tracks = tracks;
                }
                Event::Samples { track, samples } => {
                    out.samples.entry(track).or_default().extend(samples)
                }
            }
        }
    }
    assert_eq!(p.pending_bytes(), 0, "bytes left over");
    out
}

fn video(p: &Parsed) -> &Vec<Sample> {
    let id = p
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap()
        .id;
    &p.samples[&id]
}

fn audio(p: &Parsed) -> &Vec<Sample> {
    let id = p
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Audio)
        .unwrap()
        .id;
    &p.samples[&id]
}

fn check_timeline(samples: &[Sample]) {
    assert!(samples[0].key, "the first frame is a sync sample");
    for w in samples.windows(2) {
        assert!(w[1].dts > w[0].dts, "decode times increase");
    }
    for s in samples {
        assert!(s.duration > 0, "{s:?}");
        assert!(!s.data.is_empty());
    }
}

#[test]
fn h264_in_mp4() {
    let p = parse(Container::Mp4, &fixture("v_h264.mp4"), usize::MAX);
    assert_eq!(p.inits, 1);
    assert_eq!(p.tracks.len(), 1);
    let t = &p.tracks[0];
    assert_eq!(t.kind, TrackKind::Video);
    assert!(matches!(t.codec, Codec::H264 { ref avcc } if avcc.len() > 6));
    assert!(t.width > 0 && t.height > 0);
    assert!(t.codec_string().starts_with("avc1."));
    let v = video(&p);
    assert_eq!(v.len(), 60);
    check_timeline(v);
    assert!(v.iter().filter(|s| s.key).count() >= 1);
}

#[test]
fn aac_in_mp4() {
    let p = parse(Container::Mp4, &fixture("a_aac.mp4"), usize::MAX);
    let t = &p.tracks[0];
    assert_eq!(t.kind, TrackKind::Audio);
    assert!(matches!(t.codec, Codec::Aac { ref asc } if !asc.is_empty()));
    assert!(t.sample_rate >= 8000 && t.channels >= 1);
    assert_eq!(t.codec_string(), "mp4a.40.2");
    let a = audio(&p);
    assert!(a.len() > 80, "{}", a.len());
    check_timeline(a);
}

#[test]
fn audio_and_video_in_one_mp4() {
    let p = parse(Container::Mp4, &fixture("av_h264_aac.mp4"), usize::MAX);
    assert_eq!(p.tracks.len(), 2);
    assert_eq!(video(&p).len(), 60);
    check_timeline(video(&p));
    check_timeline(audio(&p));
    // Both tracks cover about the same two seconds.
    let v_end = video(&p).last().unwrap().end();
    let a_end = audio(&p).last().unwrap().end();
    assert!((v_end - a_end).abs() < 100_000_000, "{v_end} {a_end}");
}

#[test]
fn vp9_and_opus_in_webm() {
    let p = parse(Container::WebM, &fixture("av_vp9_opus.webm"), usize::MAX);
    assert_eq!(p.tracks.len(), 2);
    let v = p
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Video)
        .unwrap();
    assert!(matches!(v.codec, Codec::Vp9));
    assert!(v.width > 0);
    let a = p
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Audio)
        .unwrap();
    assert!(matches!(a.codec, Codec::Opus { ref head } if head.starts_with(b"OpusHead")));
    assert_eq!(video(&p).len(), 60);
    check_timeline(video(&p));
    check_timeline(audio(&p));
}

#[test]
fn vp8_and_vorbis_in_webm() {
    let p = parse(Container::WebM, &fixture("av_vp8_vorbis.webm"), usize::MAX);
    assert!(matches!(
        p.tracks
            .iter()
            .find(|t| t.kind == TrackKind::Video)
            .unwrap()
            .codec,
        Codec::Vp8
    ));
    let a = p
        .tracks
        .iter()
        .find(|t| t.kind == TrackKind::Audio)
        .unwrap();
    match &a.codec {
        Codec::Vorbis { headers } => {
            assert_eq!(headers.len(), 3);
            assert_eq!(headers[0][0], 1);
            assert_eq!(headers[1][0], 3);
            assert_eq!(headers[2][0], 5);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(video(&p).len(), 60);
    check_timeline(audio(&p));
}

/// The same frames however the bytes arrive.
#[test]
fn chunking_does_not_change_the_result() {
    for (name, container) in [
        ("av_h264_aac.mp4", Container::Mp4),
        ("av_vp9_opus.webm", Container::WebM),
        ("av_vp8_vorbis.webm", Container::WebM),
    ] {
        let data = fixture(name);
        let whole = parse(container, &data, usize::MAX);
        for chunk in [1, 7, 1000, 4096] {
            let split = parse(container, &data, chunk);
            assert_eq!(split.tracks, whole.tracks, "{name} chunk {chunk}");
            assert_eq!(split.samples, whole.samples, "{name} chunk {chunk}");
        }
    }
}

/// An initialization segment and then media segments, as an MSE page appends them: the
/// media segments carry no init of their own.
#[test]
fn media_segments_after_a_separate_init() {
    let data = fixture("av_h264_aac.mp4");
    // Split at the first `moof`.
    let moof = data.windows(4).position(|w| w == b"moof").unwrap() - 4;
    let mut p = Parser::new(Container::Mp4);
    let init = p.append(&data[..moof]).unwrap();
    assert!(matches!(init.as_slice(), [Event::Init { .. }]));
    let media = p.append(&data[moof..]).unwrap();
    assert!(media.iter().all(|e| matches!(e, Event::Samples { .. })));
    assert!(!media.is_empty());
    // A second media-only append works after a reset (an `abort()`), too.
    p.reset();
    let again = p.append(&data[moof..]).unwrap();
    assert_eq!(again.len(), media.len());
}

#[test]
fn garbage_is_an_error_not_a_panic() {
    let mut p = Parser::new(Container::Mp4);
    // A box claiming to be smaller than its own header.
    assert!(p.append(&[0, 0, 0, 4, b'm', b'o', b'o', b'v']).is_err());
    let mut p = Parser::new(Container::WebM);
    assert!(p.append(&[0x00, 0x00, 0x00, 0x00, 0x00]).is_err());
    // Truncated input is only pending.
    let data = fixture("v_h264.mp4");
    let mut p = Parser::new(Container::Mp4);
    p.append(&data[..data.len() / 2]).unwrap();
    assert!(p.pending_bytes() > 0);
}

#[test]
fn type_support() {
    use ferrite_mse::is_type_supported as ok;
    assert!(ok(r#"video/mp4; codecs="avc1.42E01E, mp4a.40.2""#));
    assert!(ok(r#"video/webm; codecs="vp9, opus""#));
    assert!(ok(r#"audio/mp4; codecs="mp4a.40.2""#));
    assert!(ok(r#"video/webm; codecs="vp09.00.10.08""#));
    assert!(!ok("video/mp4"));
    assert!(!ok(r#"video/x-flv; codecs="avc1.42E01E""#));
    assert!(!ok(r#"video/mp4; codecs="xyz""#));
}

/// What a page's garbage looks like: a box that claims to be too small, then noise.
#[test]
fn noise_after_a_bad_box_is_an_error_not_a_hang() {
    let mut bad = [0u8; 64];
    for (i, b) in bad.iter_mut().enumerate() {
        *b = (i * 37 % 256) as u8;
    }
    bad[..8].copy_from_slice(&[0, 0, 0, 4, b'm', b'o', b'o', b'v']);
    let mut p = Parser::new(Container::Mp4);
    assert!(p.append(&bad).is_err());
}
