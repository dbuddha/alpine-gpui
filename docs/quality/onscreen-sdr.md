# Onscreen SDR qualification

This protocol closes the physical compositor portion of Task #234 without
turning a screenshot into a performance or optical claim. It exercises one
opaque Alpine-owned AppKit window, the shipping `BGRA8Unorm_sRGB` Metal target,
the standard sRGB layer color space, Core Animation presentation, and a
ScreenCaptureKit capture normalized to explicit sRGB.

Apple documents that `CAMetalLayer` participates in color matching when its
color space is explicit, that `CGDisplayCopyColorSpace` returns the active
display-dependent ICC color space, and that ScreenCaptureKit can capture one
selected window. Screen capture requires prior user permission and does not
measure photons emitted by the panel.

Primary sources:

- [CGDisplayCopyColorSpace](https://developer.apple.com/documentation/coregraphics/cgdisplaycopycolorspace(_:))
- [SCScreenshotManager](https://developer.apple.com/documentation/screencapturekit/scscreenshotmanager)
- [SCStreamConfiguration colorSpaceName](https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration/colorspacename)
- [NSScreen backingScaleFactor](https://developer.apple.com/documentation/appkit/nsscreen/backingscalefactor)

## Required hardware and state

- Apple Silicon on macOS 15 or newer
- two real active displays with distinct display identities and backing scales
- Screen Recording permission already granted to the invoking terminal
- one clean Alpine revision with the checked-in Metal library
- SDR capture only, with HDR, EDR, transparency, mirroring, and virtual displays
  excluded

The command fails when only one display exists or both displays expose the same
backing scale. Synthetic scale or display-identity injection cannot satisfy
this protocol.

## Workload and negative control

The scene contains five full-height grayscale patches with linear values 0,
0.18, 0.5, 0.75, and 1. The accepted path supplies those values to the shipping
sRGB target. The deliberate control supplies their sRGB-encoded values to that
same target, producing a second transfer conversion. No alternate or wrong
rendering pipeline enters shipping code.

After explicit sRGB capture, accepted byte oracles are 0, 118, 188, 225, and
255. Each accepted patch permits at most 12 byte values of error. The wrong
control must be at least 30 byte values away from the accepted oracle on one
patch and within 12 of its independent double-encoded oracle. These thresholds
are correctness tolerances, not performance budgets.

## Required stages

1. Launch and capture on the first display.
2. Resize the real content area, present a new revision, and capture again on
   the same display and backing scale.
3. Move the owned window to a second physical display with a different backing
   scale, present a new revision, and capture again.
4. Present and capture the deliberate wrong-transfer scene on the second
   display.

Every stage retains the repository revision, OS build, hardware model, scene
revision and hash, target and layer color identities, nonzero presented-time
bits, logical and captured geometry, screen-capture permission, window and
display identities, display count, backing scale, raw PNG hash, active display
ICC profile and hash, patch samples, oracle errors, and explicit absence of a
performance claim.

## Command and retention

Run:

```sh
scripts/qualify-onscreen-sdr.sh /absolute/output/directory
```

The output directory must not already exist. The command compiles the
non-shipping ScreenCaptureKit helper, runs its pure transfer-control self-test,
executes the native driver, validates the complete bundle with
`alpine-assurance`, and writes `report.md`. Raw captures and ICC profiles remain
part of the revision-scoped artifact bundle. They are not committed by default
because display profiles can contain device-specific identifiers; the accepted
issue comment or release evidence stores the bundle hash and controlled
retention location.

## Invalid evidence

Reject the bundle for missing permission, dirty source, missing or duplicate
stage, synthetic transition, unchanged display identity or backing scale,
unpresented frame, target or color-space drift, EDR, artifact hash mismatch,
profile omission, accepted-patch tolerance failure, nondiscriminating negative
control, or any performance claim. A passing bundle establishes compositor
pixel correctness for the named displays and revision only. It does not prove
optical appearance, HDR behavior, universal color correctness, latency, or
performance superiority.
