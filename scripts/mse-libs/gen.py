"""Splits the fragmented MP4 files in DIR (v.mp4, a.mp4, av.mp4) into initialization and media
segments and writes a DASH manifest (manifest.mpd) and an HLS playlist (master.m3u8, av.m3u8)."""
import struct, sys, os
def boxes(d, i=0, end=None):
    end = len(d) if end is None else end
    while i + 8 <= end:
        sz, = struct.unpack(">I", d[i:i+4]); t = d[i+4:i+8].decode()
        yield t, i, sz
        i += sz
def child(d, start, size, name):
    for t, i, sz in boxes(d, start + 8, start + size):
        if t == name: return i, sz
def timescales(d):
    ts = {}
    for t, i, sz in boxes(d):
        if t == 'moov':
            for t2, j, s2 in boxes(d, i + 8, i + sz):
                if t2 == 'trak':
                    tkhd = child(d, j, s2, 'tkhd'); tv = d[tkhd[0]+8]; tid, = struct.unpack('>I', d[tkhd[0]+8+4+(8 if tv == 0 else 16):tkhd[0]+8+4+(8 if tv == 0 else 16)+4])
                    mdia = child(d, j, s2, 'mdia'); mdhd = child(d, mdia[0], mdia[1], 'mdhd')
                    v = d[mdhd[0]+8]; off = mdhd[0] + 8 + 4 + (8 if v == 0 else 16)
                    ts[tid], = struct.unpack(">I", d[off:off+4])
    return ts
def moof_info(d, i, sz):
    out = {}
    for t, j, s in boxes(d, i + 8, i + sz):
        if t == 'traf':
            tfhd = child(d, j, s, 'tfhd'); fl = struct.unpack(">I", b'\0' + d[tfhd[0]+9:tfhd[0]+12])[0]
            tid, = struct.unpack(">I", d[tfhd[0]+12:tfhd[0]+16]); p = tfhd[0] + 16
            if fl & 1: p += 8
            if fl & 2: p += 4
            ddur = None
            if fl & 8: ddur, = struct.unpack(">I", d[p:p+4])
            tfdt = child(d, j, s, 'tfdt'); v = d[tfdt[0]+8]
            base, = struct.unpack(">Q", d[tfdt[0]+12:tfdt[0]+20]) if v == 1 else struct.unpack(">I", d[tfdt[0]+12:tfdt[0]+16])
            trun = child(d, j, s, 'trun'); tf = struct.unpack(">I", b'\0' + d[trun[0]+9:trun[0]+12])[0]
            n, = struct.unpack(">I", d[trun[0]+12:trun[0]+16]); q = trun[0] + 16
            if tf & 1: q += 4
            if tf & 4: q += 4
            dur = 0
            for _ in range(n):
                sd = ddur or 0
                if tf & 0x100: sd, = struct.unpack(">I", d[q:q+4]); q += 4
                if tf & 0x200: q += 4
                if tf & 0x400: q += 4
                if tf & 0x800: q += 4
                dur += sd
            out[tid] = (base, dur)
    return out
def split(path, prefix, outdir, group):
    d = open(path, "rb").read(); ts = timescales(d)
    bl = list(boxes(d)); init_end = next(i for t, i, sz in bl if t == 'moof')
    open(f"{outdir}/init_{prefix}.mp4", "wb").write(d[:init_end])
    frags = []; cur = None
    for t, i, sz in bl:
        if t == 'moof': cur = [i, i + sz, moof_info(d, i, sz)]
        elif t == 'mdat' and cur:
            cur[1] = i + sz; frags.append(cur); cur = None
    segs = []
    for k in range(0, len(frags), group):
        part = frags[k:k+group]; segs.append((part[0][0], part[-1][1], part[0][2], part[-1][2]))
    result = []
    for n, (a, b, info, _) in enumerate(segs, 1):
        open(f"{outdir}/{prefix}_{n}.m4s", "wb").write(d[a:b]); result.append(info)
    return ts, result
outdir = sys.argv[1]
tsv, vsegs = split(f"{outdir}/v.mp4", "v", outdir, 1)
tsa, asegs = split(f"{outdir}/a.mp4", "a", outdir, 1)
tsm, msegs = split(f"{outdir}/av.mp4", "av", outdir, 2)
vt = list(tsv.values())[0]; at = list(tsa.values())[0]
def timeline(segs, ts):
    return "".join(f'<S t="{list(info.values())[0][0]}" d="{list(info.values())[0][1]}"/>' for info in segs)
vdur = sum(list(i.values())[0][1] for i in vsegs) / vt
adur = sum(list(i.values())[0][1] for i in asegs) / at
mpd = f'''<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-on-demand:2011" type="static" mediaPresentationDuration="PT{vdur:.3f}S" minBufferTime="PT2S">
<Period>
<AdaptationSet contentType="video" mimeType="video/mp4" segmentAlignment="true"><Representation id="v" codecs="avc1.42c00d" width="320" height="240" bandwidth="400000"><SegmentTemplate timescale="{vt}" initialization="init_v.mp4" media="v_$Number$.m4s" startNumber="1"><SegmentTimeline>{timeline(vsegs, vt)}</SegmentTimeline></SegmentTemplate></Representation></AdaptationSet>
<AdaptationSet contentType="audio" mimeType="audio/mp4" lang="en"><Representation id="a" codecs="mp4a.40.2" audioSamplingRate="44100" bandwidth="64000"><SegmentTemplate timescale="{at}" initialization="init_a.mp4" media="a_$Number$.m4s" startNumber="1"><SegmentTimeline>{timeline(asegs, at)}</SegmentTimeline></SegmentTemplate></Representation></AdaptationSet>
</Period></MPD>'''
open(f"{outdir}/manifest.mpd", "w").write(mpd)
# HLS, muxed segments of two fragments (video and audio) each.
lines = ["#EXTM3U", "#EXT-X-VERSION:7", "#EXT-X-TARGETDURATION:3", "#EXT-X-MEDIA-SEQUENCE:1", "#EXT-X-PLAYLIST-TYPE:VOD", '#EXT-X-MAP:URI="init_av.mp4"']
for n, info in enumerate(msegs, 1):
    # the video track (the one with 30 fps ticks) decides the length
    dur = max(d / tsm[t] for t, (b, d) in info.items())
    lines += [f"#EXTINF:{dur:.5f},", f"av_{n}.m4s"]
lines.append("#EXT-X-ENDLIST")
open(f"{outdir}/av.m3u8", "w").write("\n".join(lines) + "\n")
master = '#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=320x240,CODECS="avc1.42c00d,mp4a.40.2"\nav.m3u8\n'
open(f"{outdir}/master.m3u8", "w").write(master)
print("video", vdur, "audio", adur, len(vsegs), len(asegs), len(msegs))
