# Privacy boundary

SurfaceCheck is local-only by default. The capture and review pipeline does
not contain an HTTP client, vendor SDK, telemetry path, or automatic upload.
The optional Premonition adapter is injected by the operator and is disabled
when its exact versioned contract is unavailable.

The following remain local unless the operator explicitly chooses an external
handoff:

- PNG bytes and derived previews;
- window geometry and the capture timestamp;
- user notes and deterministic findings;
- agent prompts, responses and findings;
- before/after comparisons and exported archives.

Raw window titles, URLs, class names, full paths, EDID data, serials and raw
tool stderr are not persisted. A capture may carry only a bounded,
user-supplied redacted application alias. Routine logs contain operation IDs,
stable error codes, durations, sizes and counts rather than content.

An evidence reference carries the capture ID, checksum and a rectangle. The
manifest validator resolves the ID, verifies the checksum and rejects any
rectangle outside the image. Export is an explicit local file operation; the
archive is not sent anywhere by SurfaceCheck.

Users should treat an explicitly selected capture and any consented external
handoff as sensitive. Remove the private local bundle using the operator's
normal filesystem policy when it is no longer needed.
