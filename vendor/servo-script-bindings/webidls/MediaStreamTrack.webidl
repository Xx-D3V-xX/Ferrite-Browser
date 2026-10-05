/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

// https://w3c.github.io/mediacapture-main/#dom-mediastreamtrack

[Exposed=Window]
interface MediaStreamTrack : EventTarget {
    readonly        attribute DOMString kind;
    readonly        attribute DOMString id;
    readonly        attribute DOMString label;
                    attribute boolean enabled;
    readonly        attribute boolean muted;
                    attribute EventHandler onmute;
                    attribute EventHandler onunmute;
    readonly        attribute MediaStreamTrackState readyState;
                    attribute EventHandler onended;
    MediaStreamTrack clone();
    undefined stop();
    MediaTrackSettings getSettings();
};

// Ferrite: the track API the engine had left commented out.
enum MediaStreamTrackState {
    "live",
    "ended"
};

dictionary MediaTrackSettings {
    DOMString deviceId;
    DOMString groupId;
    long width;
    long height;
    double aspectRatio;
    double frameRate;
    long sampleRate;
    long channelCount;
    DOMString displaySurface;
};
