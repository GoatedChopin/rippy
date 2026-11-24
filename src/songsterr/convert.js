// Songsterr's song data -> a Guitar Pro 7 file, written by alphaTab's Gp7Exporter.
//
// Rippy runs this in QuickJS (src/songsterr.rs) after shims.js and vendor/alphatab/alphaTab.min.js, the
// alphaTab build Riffalong ships. It defines one function:
//
//   songsterrToGp(metaJson, partsJson) -> { gp: Uint8Array, report: string (JSON) }
//
// metaJson is /api/meta/{songId}[/{revisionId}]; partsJson is an array with one CDN track file per
// track, in order. After writing the file it reads it back and compares every note, so the report
// says what made it into the .gp.
(function () {
  'use strict';

  const at = globalThis.alphaTab;
  const M = at.model;

  const BEAT_KEYS = new Set(['notes', 'type', 'duration', 'dots', 'dotted', 'tuplet', 'tupletStart', 'tupletStop', 'rest', 'beamStart', 'beamStop',
    'velocity', 'gradualVelocity', 'chord', 'text', 'letRing', 'palmMute', 'graceNote', 'upStroke', 'downStroke', 'brushStroke', 'arpeggio',
    'upArpeggio', 'downArpeggio', 'pickStroke', 'vibratoWithTremoloBar', 'tremoloBar', 'tapping', 'slapping', 'popping', 'fadeIn', 'fadding', 'width', 'voice']);
  const NOTE_KEYS = new Set(['fret', 'string', 'rest', 'tie', 'hp', 'ghost', 'dead', 'staccato', 'accentuated', 'vibrato', 'wideVibrato', 'leftHandVibrato',
    'bend', 'slide', 'harmonic', 'harmonicFret', 'trill', 'leftFingering', 'rightFingering', 'velocity']);
  const MEASURE_KEYS = new Set(['voices', 'signature', 'marker', 'rest', 'repeatStart', 'repeat', 'alternateEnding', 'doubleBarline', 'freeTime', 'direction',
    'tripletFeel', 'clef', 'index', 'scale']);

  const DYN = { ppp: M.DynamicValue.PPP, pp: M.DynamicValue.PP, p: M.DynamicValue.P, mp: M.DynamicValue.MP, mf: M.DynamicValue.MF, f: M.DynamicValue.F, ff: M.DynamicValue.FF, fff: M.DynamicValue.FFF };
  const HARM = { true: M.HarmonicType.Natural, natural: M.HarmonicType.Natural, artificial: M.HarmonicType.Artificial, pinch: M.HarmonicType.Pinch,
    tapped: M.HarmonicType.Tap, semi: M.HarmonicType.Semi, feedback: M.HarmonicType.Feedback };
  const LEFT = { 0: M.Fingers.Thumb, T: M.Fingers.Thumb, 1: M.Fingers.IndexFinger, 2: M.Fingers.MiddleFinger, 3: M.Fingers.AnnularFinger, 4: M.Fingers.LittleFinger };
  const CLEF = { G2: M.Clef.G2, F4: M.Clef.F4, C3: M.Clef.C3, C4: M.Clef.C4 };
  const RIGHT = { P: M.Fingers.Thumb, I: M.Fingers.IndexFinger, M: M.Fingers.MiddleFinger, A: M.Fingers.AnnularFinger, C: M.Fingers.LittleFinger };
  const DIR = { coda: 'TargetCoda', doublecoda: 'TargetDoubleCoda', segno: 'TargetSegno', segnosegno: 'TargetSegnoSegno', fine: 'TargetFine',
    dacapo: 'JumpDaCapo', dasegno: 'JumpDalSegno', dasegnosegno: 'JumpDalSegnoSegno', dacapoalcoda: 'JumpDaCapoAlCoda', dasegnoalcoda: 'JumpDalSegnoAlCoda',
    dasegnosegnoalcoda: 'JumpDalSegnoSegnoAlCoda', dacapoaldoublecoda: 'JumpDaCapoAlDoubleCoda', dasegnoaldoublecoda: 'JumpDalSegnoAlDoubleCoda',
    dasegnosegnoaldoublecoda: 'JumpDalSegnoSegnoAlDoubleCoda', dacoda: 'JumpDaCoda', dadoublecoda: 'JumpDaDoubleCoda', dacapoalfine: 'JumpDaCapoAlFine',
    dasegnoalfine: 'JumpDalSegnoAlFine', dasegnosegnoalfine: 'JumpDalSegnoSegnoAlFine' };
  // Songsterr's drum numbers are Guitar Pro 5's; these five differ in Guitar Pro 7. Same as alphaTab's GP5 importer.
  const GP5_DRUM_REMAP = new Map([[27, 42], [28, 60], [29, 29], [30, 30], [32, 31]]);

  // alphaTab's default drum kit. PercussionMapper isn't exported, so read it back from a one-bar drum
  // score: the GP7 writer emits the full default kit when a track has none.
  let drumKit = null;
  function defaultDrumKit() {
    if (drumKit) return drumKit;
    const s = new M.Score(); const mb = new M.MasterBar(); s.addMasterBar(mb);
    const t = new M.Track(); t.ensureStaveCount(1); t.staves[0].isPercussion = true; t.playbackInfo.primaryChannel = 9; s.addTrack(t);
    const bar = new M.Bar(); t.staves[0].addBar(bar); const v = new M.Voice(); bar.addVoice(v); const b = new M.Beat(); b.duration = 1; v.addBeat(b);
    const st = new at.Settings(); s.finish(st);
    drumKit = at.importer.ScoreLoader.loadScoreFromBytes(new at.exporter.Gp7Exporter().export(s, st), st).tracks[0].percussionArticulations;
    return drumKit;
  }

  globalThis.songsterrToGp = function (metaJson, partsJson) {
    const meta = JSON.parse(metaJson);
    const parts = JSON.parse(partsJson);
    if (!parts.length) throw new Error('The song has no tracks');
    const { songId, revisionId } = meta;
    const DRUM_KIT = defaultDrumKit();

    const unmapped = {}; // Songsterr fields this doesn't carry over, with counts
    const miss = (k) => { unmapped[k] = (unmapped[k] ?? 0) + 1; };

    let score = new M.Score();
    score.title = meta.title ?? '';
    score.artist = meta.artist ?? '';
    score.tab = `${meta.author?.name ?? ''} (Songsterr song ${songId}, revision ${revisionId})`.trim();

    const barCount = Math.max(...parts.map((p) => p.measures?.length ?? 0));
    let sig = [4, 4];
    for (let b = 0; b < barCount; b++) {
      const mb = new M.MasterBar();
      // Bar-level facts sit on every part's copy of the bar; take the first part that has each one.
      const m = parts.map((p) => p.measures?.[b] ?? {}).reduce((acc, x) => ({ ...x, ...acc }), {});
      for (const k of Object.keys(m)) if (!MEASURE_KEYS.has(k)) miss(`measure.${k}`);
      if (m.signature) sig = m.signature;
      mb.timeSignatureNumerator = sig[0];
      mb.timeSignatureDenominator = sig[1];
      if (m.marker?.text) { const s = new M.Section(); s.text = m.marker.text; s.marker = ''; mb.section = s; }
      if (m.repeatStart) mb.isRepeatStart = true;
      if (m.repeat) mb.repeatCount = m.repeat;
      if (Array.isArray(m.alternateEnding)) mb.alternateEndings = m.alternateEnding.reduce((acc, n) => acc | (1 << (n - 1)), 0);
      if (m.doubleBarline) mb.isDoubleBar = true;
      if (m.freeTime) mb.isFreeTime = true;
      for (const t of [...(m.direction?.target ?? []), ...(m.direction?.jump ?? [])]) if (DIR[t]) mb.addDirection(M.Direction[DIR[t]]); else miss(`direction.${t}`);
      if (m.tripletFeel) miss('measure.tripletFeel');
      score.addMasterBar(mb);
    }

    // Tempo: Songsterr keeps the song's tempo map on every part.
    for (const t of parts[0].automations?.tempo ?? []) {
      const mb = score.masterBars[t.measure];
      if (!mb) continue;
      const quarterBpm = t.bpm * (4 / (t.type || 4)) * (t.dotted ? 1.5 : 1);
      const beats = parts[0].measures[t.measure]?.voices?.[0]?.beats ?? [];
      let ratio = 0;
      if (t.position > 0 && beats.length) { // position = beat index inside the bar
        let acc = 0; for (let i = 0; i < Math.min(t.position, beats.length); i++) acc += beats[i].duration[0] / beats[i].duration[1];
        ratio = Math.min(0.999, acc / (mb.timeSignatureNumerator / mb.timeSignatureDenominator));
      }
      mb.tempoAutomations.push(M.Automation.buildTempoAutomation(!!t.linear, ratio, quarterBpm, 2));
    }

    let nextChannel = 0;
    const allocChannel = () => { const c = nextChannel % 16; nextChannel++; return c === 9 ? allocChannel() : c; };

    let barMismatches = 0;
    parts.forEach((p, pi) => {
      const isDrums = p.instrumentId === 1024;
      const track = new M.Track();
      track.name = p.name || meta.tracks?.[pi]?.instrument || `Track ${pi + 1}`;
      track.shortName = track.name.split('|').pop().trim().slice(0, 10);
      track.ensureStaveCount(1);
      const staff = track.staves[0];
      if (isDrums) {
        staff.isPercussion = true;
        staff.showTablature = false;
        track.playbackInfo.primaryChannel = 9; track.playbackInfo.secondaryChannel = 9;
      } else {
        track.playbackInfo.program = Math.max(0, Math.min(127, p.instrumentId ?? 25));
        track.playbackInfo.primaryChannel = allocChannel(); track.playbackInfo.secondaryChannel = allocChannel();
        staff.stringTuning = new M.Tuning('', (p.tuning ?? [64, 59, 55, 50, 45, 40]).slice(), false);
        if (p.capo) staff.capo = p.capo;
      }
      track.playbackInfo.volume = Math.round(15 * (p.volume ?? 1));
      track.playbackInfo.balance = Math.round(8 + 8 * (p.balance ?? 0));
      const stringCount = p.strings ?? p.tuning?.length ?? 6;
      const chordIds = new Map();
      const drumIndex = new Map(); // articulation id -> index in this track's list
      let clef = M.Clef.G2; // Songsterr gives the clef where it changes; it holds until the next one
      score.addTrack(track);

      for (let b = 0; b < barCount; b++) {
        const bar = new M.Bar();
        staff.addBar(bar);
        const measure = p.measures?.[b] ?? { voices: [] };
        if (measure.clef) {
          if (CLEF[measure.clef] !== undefined) clef = CLEF[measure.clef];
          else miss(`clef.${measure.clef}`);
        }
        if (!isDrums) bar.clef = clef;
        const voices = measure.voices?.length ? measure.voices : [{ beats: [] }];
        voices.forEach((v, vi) => {
          const voice = new M.Voice();
          bar.addVoice(voice);
          let total = 0;
          for (const sb of v.beats ?? []) {
            for (const k of Object.keys(sb)) if (!BEAT_KEYS.has(k)) miss(`beat.${k}`);
            const beat = new M.Beat();
            beat.duration = sb.type ?? 4;
            beat.dots = sb.dots ?? (sb.dotted ? 1 : 0);
            const real = sb.duration ? sb.duration[0] / sb.duration[1] : 0;
            if (sb.tuplet) {
              // Songsterr gives the real length in `duration`; derive the tuplet's denominator from it.
              const nominal = (1 / beat.duration) * (2 - 1 / 2 ** beat.dots);
              beat.tupletNumerator = sb.tuplet;
              beat.tupletDenominator = Math.round((sb.tuplet * real) / nominal) || 2;
            }
            if (!sb.graceNote) total += real; // grace notes take no time in the bar
            if (sb.velocity && DYN[sb.velocity] !== undefined) beat.dynamics = DYN[sb.velocity];
            if (sb.gradualVelocity) beat.crescendo = sb.gradualVelocity === 'crescendo' ? M.CrescendoType.Crescendo : M.CrescendoType.Decrescendo;
            if (sb.text?.text) beat.text = sb.text.text;
            if (sb.graceNote) beat.graceType = sb.graceNote === 'onBeat' ? M.GraceType.OnBeat : M.GraceType.BeforeBeat;
            if (sb.upStroke || sb.brushStroke?.direction === 'up') beat.brushType = M.BrushType.BrushUp;
            if (sb.downStroke || sb.brushStroke?.direction === 'down') beat.brushType = M.BrushType.BrushDown;
            if (sb.arpeggio || sb.upArpeggio || sb.downArpeggio) beat.brushType = (sb.arpeggio?.direction === 'down' || sb.downArpeggio) ? M.BrushType.ArpeggioDown : M.BrushType.ArpeggioUp;
            if (sb.brushStroke?.duration || sb.arpeggio?.duration) beat.brushDuration = sb.brushStroke?.duration ?? sb.arpeggio?.duration;
            if (sb.pickStroke) beat.pickStroke = sb.pickStroke === 'up' ? M.PickStroke.Up : M.PickStroke.Down;
            if (sb.vibratoWithTremoloBar) beat.vibrato = sb.vibratoWithTremoloBar === 'wide' ? M.VibratoType.Wide : M.VibratoType.Slight;
            if (sb.tremoloBar?.points) for (const pt of sb.tremoloBar.points) beat.addWhammyBarPoint(new M.BendPoint(pt.position, pt.tone / 25));
            if (sb.tapping) beat.tap = true;
            if (sb.slapping) beat.slap = true;
            if (sb.popping) beat.pop = true;
            if (sb.fadeIn || sb.fadding === 'fadein') beat.fade = M.FadeType.FadeIn;
            else if (sb.fadding === 'fadeout') beat.fade = M.FadeType.FadeOut;
            else if (sb.fadding === 'volumeswell') beat.fade = M.FadeType.VolumeSwell;
            if (sb.chord?.text && !isDrums) {
              if (!chordIds.has(sb.chord.text)) {
                const c = new M.Chord(); c.name = sb.chord.text; c.showDiagram = false; c.showName = true;
                const id = String(chordIds.size); chordIds.set(sb.chord.text, id); staff.addChord(id, c);
              }
              beat.chordId = chordIds.get(sb.chord.text);
            }
            if (!sb.rest) {
              for (const sn of sb.notes ?? []) {
                if (sn.rest) continue;
                for (const k of Object.keys(sn)) if (!NOTE_KEYS.has(k)) miss(`note.${k}`);
                const note = new M.Note();
                if (isDrums) {
                  const id = GP5_DRUM_REMAP.get(sn.fret) ?? sn.fret;
                  if (!drumIndex.has(id)) {
                    const art = DRUM_KIT.find((a) => a.id === id) ?? DRUM_KIT.find((a) => a.outputMidiNumber === id);
                    if (!art) { miss(`drum.${id}`); continue; }
                    drumIndex.set(id, track.percussionArticulations.length); track.percussionArticulations.push(art);
                  }
                  note.percussionArticulation = drumIndex.get(id);
                } else {
                  note.fret = sn.fret ?? 0;
                  note.string = stringCount - sn.string; // Songsterr: 0 = highest string. alphaTab: 1 = lowest.
                }
                if (sn.tie) note.isTieDestination = true;
                if (sn.hp) note.isHammerPullOrigin = true; // Songsterr marks the first note of the pair, as Guitar Pro does
                if (sn.ghost) note.isGhost = true;
                if (sn.dead) note.isDead = true;
                if (sn.staccato) note.isStaccato = true;
                if (sn.accentuated) note.accentuated = sn.accentuated >= 2 ? M.AccentuationType.Heavy : M.AccentuationType.Normal;
                if (sn.vibrato || sn.leftHandVibrato === 'slight') note.vibrato = M.VibratoType.Slight;
                if (sn.wideVibrato || sn.leftHandVibrato === 'wide') note.vibrato = M.VibratoType.Wide;
                if (sn.bend?.points) for (const pt of sn.bend.points) note.addBendPoint(new M.BendPoint(pt.position, pt.tone / 25)); // 100 = full step
                if (sn.slide) {
                  const s = String(sn.slide);
                  if (s.startsWith('below')) note.slideInType = M.SlideInType.IntoFromBelow;
                  if (s.startsWith('above')) note.slideInType = M.SlideInType.IntoFromAbove;
                  const out = s.replace(/^(below|above)/, '');
                  if (out === 'shift') note.slideOutType = M.SlideOutType.Shift;
                  if (out === 'legato') note.slideOutType = M.SlideOutType.Legato;
                  if (out === 'downwards') note.slideOutType = M.SlideOutType.OutDown;
                  if (out === 'upwards') note.slideOutType = M.SlideOutType.OutUp;
                }
                if (sn.harmonic) { note.harmonicType = HARM[sn.harmonic] ?? M.HarmonicType.Natural; if (sn.harmonicFret != null) note.harmonicValue = sn.harmonicFret; }
                if (sn.trill) { note.trillValue = sn.trill.auxiliaryFret; note.trillSpeed = M.Duration.Sixteenth; }
                if (sn.leftFingering != null && LEFT[sn.leftFingering] !== undefined) note.leftHandFinger = LEFT[sn.leftFingering];
                if (sn.rightFingering && RIGHT[sn.rightFingering] !== undefined) note.rightHandFinger = RIGHT[sn.rightFingering];
                if (sb.letRing) note.isLetRing = true;
                if (sb.palmMute) note.isPalmMute = true;
                beat.addNote(note);
              }
            }
            voice.addBeat(beat);
          }
          if (!voice.beats.length) { const r = new M.Beat(); r.duration = M.Duration.Whole; if (vi > 0) r.isEmpty = true; voice.addBeat(r); }
          const mbSig = score.masterBars[b];
          if (v.beats?.length && Math.abs(total - mbSig.timeSignatureNumerator / mbSig.timeSignatureDenominator) > 1e-6) barMismatches++;
        });
      }
    });

    const settings = new at.Settings();
    score.finish(settings);

    // Lyrics: Songsterr stores Guitar Pro-style lines (text + start bar) on vocal parts. Dispatched after
    // finish() so beats are linked: one syllable per sung beat, skipping rests and beats whose notes are all
    // tied over. Empty chunks are section breaks.
    let lyricsWritten = 0;
    parts.forEach((p, pi) => {
      const lines = (p.newLyrics ?? []).filter((l) => l.text?.trim());
      lines.forEach((l, li) => {
        const ly = new M.Lyrics(); ly.startBar = Math.max(0, (l.offset ?? 1) - 1); ly.text = l.text; ly.finish();
        const chunks = ly.chunks.filter((c) => c.trim());
        const bars = score.tracks[pi].staves[0].bars;
        let i = 0;
        for (let b = ly.startBar; b < bars.length && i < chunks.length; b++) for (const beat of bars[b].voices[0].beats) {
          if (i >= chunks.length) break;
          if (beat.isEmpty || beat.isRest || beat.notes.every((n) => n.isTieDestination)) continue;
          if (!beat.lyrics) beat.lyrics = new Array(lines.length).fill('');
          beat.lyrics[li] = chunks[i++]; lyricsWritten++;
        }
      });
    });

    const gp = new at.exporter.Gp7Exporter().export(score, settings);
    const title = score.title;
    const artist = score.artist;
    const sections = score.masterBars.filter((mb) => mb.section).map((mb) => ({ bar: mb.index + 1, text: mb.section.text }));
    score = null; // the copy read back below takes as much memory again

    // Read the file back and compare every note's pitch, string and fret (drums: the articulation).
    const back = at.importer.ScoreLoader.loadScoreFromBytes(gp, settings);
    const tracks = parts.map((p, pi) => {
      const isDrums = p.instrumentId === 1024;
      const t = back.tracks[pi];
      const st = t.staves[0];
      let notes = 0, matched = 0;
      for (let b = 0; b < barCount; b++) {
        (p.measures?.[b]?.voices ?? []).forEach((v, vi) => {
          const beatsBack = st.bars[b].voices[vi]?.beats ?? [];
          (v.beats ?? []).forEach((sb, bi) => {
            if (sb.rest) return;
            const want = (sb.notes ?? []).filter((n) => !n.rest).map((n) => isDrums
              ? `d${GP5_DRUM_REMAP.get(n.fret) ?? n.fret}`
              : `${(p.tuning?.[n.string] ?? 0) + (n.fret ?? 0)}:${n.string}:${n.fret ?? 0}`).sort();
            const got = (beatsBack[bi]?.notes ?? []).map((n) => isDrums
              ? `d${t.percussionArticulations[n.percussionArticulation]?.id ?? -1}`
              : `${st.tuning[st.tuning.length - n.string] + n.fret}:${st.tuning.length - n.string}:${n.fret}`).sort();
            notes += want.length;
            matched += want.filter((x, i) => x === got[i]).length;
          });
        });
      }
      return {
        name: t.name,
        instrument: meta.tracks?.[pi]?.instrument ?? null,
        drums: isDrums,
        strings: isDrums ? null : st.tuning.length,
        tuning: isDrums ? null : st.tuning.slice(),
        capo: isDrums ? null : st.capo,
        notes,
        matched,
      };
    });
    const lyricsReadBack = back.tracks.reduce((a, t) => a + t.staves[0].bars.reduce((x, bar) =>
      x + bar.voices.reduce((y, v) => y + v.beats.filter((bt) => bt.lyrics?.some((q) => q)).length, 0), 0), 0);

    const report = {
      songId,
      revisionId,
      title,
      artist,
      bars: barCount,
      notes: tracks.reduce((a, t) => a + t.notes, 0),
      matched: tracks.reduce((a, t) => a + t.matched, 0),
      lyrics: { written: lyricsWritten, readBack: lyricsReadBack },
      sections,
      barsNotFull: barMismatches,
      notCarriedOver: unmapped,
      tracks,
    };
    return { gp, report: JSON.stringify(report) };
  };
})();
