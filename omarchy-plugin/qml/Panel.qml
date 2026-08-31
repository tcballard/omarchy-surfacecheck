import QtQuick
import Quickshell
import Quickshell.Io

Item {
    id: root
    visible: false
    width: 680
    height: 760
    focus: true

    property string payloadJson: "{}"
    property string captureSummary: "No capture yet."
    property string deterministicFindings: "Deterministic findings will appear here."
    property string agentFindings: "Agent review is disabled until explicitly requested."
    property string comparisonSummary: "No before/after comparison yet."
    property string evidenceSummary: "Evidence stays local until you explicitly export it."
    property string userNote: ""
    property string applicationAddress: ""
    property string findingId: ""
    property string beforeCaptureId: ""
    property string afterCaptureId: ""

    function open(payload) {
        payloadJson = String(payload || "{}").slice(0, 65536)
        visible = true
        forceActiveFocus()
        if (payloadJson !== "{}") {
            handleResponse("capture", payloadJson)
        }
    }

    function close() {
        visible = false
    }

    function bounded(value, maximum) {
        return String(value || "").slice(0, maximum)
    }

    function safe(value, maximum) {
        var text = bounded(value, maximum)
        // The CLI already omits paths and raw stderr. Keep the display safe if
        // a future adapter returns an unexpected absolute path in free text.
        return text.replace(/(^|[\s])\/(?:[^\s]+)/g, "$1[redacted-path]")
    }

    function responseFailure(kind, response) {
        var message = kind + ": " + bounded(response.status, 32)
        if (response.error && response.error.message) {
            message += " — " + safe(response.error.message, 512)
        }
        return message
    }

    function formatFindings(findings, emptyText) {
        if (!Array.isArray(findings) || findings.length === 0) {
            return emptyText
        }
        var lines = []
        var limit = Math.min(findings.length, 16)
        for (var index = 0; index < limit; index += 1) {
            var finding = findings[index] || {}
            var evidence = Array.isArray(finding.evidence) ? finding.evidence.length : 0
            var line = bounded(finding.findingId, 96) + " ["
                    + bounded(finding.category, 32) + "/"
                    + bounded(finding.severity, 16) + "] "
                    + evidence + " evidence region(s)"
            if (finding.explanation) {
                line += ": " + safe(finding.explanation, 512)
            }
            if (finding.confidence !== undefined) {
                line += " (confidence " + Number(finding.confidence).toFixed(2) + ")"
            }
            lines.push(line)
        }
        if (findings.length > limit) {
            lines.push("… " + (findings.length - limit) + " more finding(s) omitted")
        }
        return lines.join("\n").slice(0, 16384)
    }

    function handleResponse(kind, rawText) {
        var text = bounded(rawText, 65536)
        var response
        try {
            response = JSON.parse(text)
        } catch (error) {
            captureSummary = kind + ": malformed CLI response"
            return
        }
        if (!response || response.schemaVersion !== 1 || !response.status) {
            captureSummary = kind + ": invalid schemaVersion 1 response"
            return
        }
        if (response.status !== "success" || !response.result) {
            var failure = responseFailure(kind, response)
            if (kind === "review") {
                agentFindings = failure
            } else if (kind === "export" || kind === "handoff") {
                evidenceSummary = failure
            } else if (kind === "compare" || kind === "select") {
                comparisonSummary = failure
            } else {
                captureSummary = failure
            }
            return
        }
        var result = response.result
        if (kind === "capture") {
            var dimensions = result.dimensions || {}
            var width = result.width || dimensions.width || "?"
            var height = result.height || dimensions.height || "?"
            captureSummary = "captureId=" + bounded(result.captureId, 96)
                    + " type=" + bounded(result.captureType, 24)
                    + " dimensions=" + width + "×" + height
                    + " scale=" + JSON.stringify(result.scale || {})
                    + " sha256=" + bounded(result.sha256, 64)
                    + " stored=" + Boolean(result.stored)
                    + " stale=" + Boolean(result.stale)
        } else if (kind === "review") {
            deterministicFindings = formatFindings(
                result.deterministicFindings,
                "No deterministic findings returned.")
            agentFindings = formatFindings(
                result.agentFindings,
                result.agentStatus === "unavailable"
                    ? "Agent review is unavailable or not configured."
                    : "No agent findings returned.")
        } else if (kind === "compare" || kind === "select") {
            comparisonSummary = "comparisonId=" + bounded(result.comparisonId, 96)
                    + " changed=" + Number(result.changedFraction || 0).toFixed(4)
                    + " mean=" + Number(result.meanAbsoluteDifference || 0).toFixed(4)
                    + " rms=" + Number(result.rmsDifference || 0).toFixed(4)
                    + " perceptual=" + Number(result.perceptualDistance || 0).toFixed(4)
        } else if (kind === "annotate") {
            evidenceSummary = result.annotated ? "User note saved locally." : "User note was not saved."
        } else if (kind === "export") {
            evidenceSummary = "Local USTAR bundle: " + bounded(result.relativePath, 256)
                    + " (" + bounded(result.bytes, 24) + " bytes, sha256="
                    + bounded(result.sha256, 64) + ")"
        } else if (kind === "handoff") {
            evidenceSummary = "Premonition handoff: " + bounded(result.status, 32)
                    + (result.externalReference
                        ? " (reference=" + bounded(result.externalReference, 128) + ")"
                        : "")
        }
    }

    Process {
        id: captureWindow
        command: ["surfacecheck", "capture", "window", "--json"]
        stdout: StdioCollector { onStreamFinished: root.handleResponse("capture", this.text) }
    }
    Process {
        id: captureRegion
        command: ["surfacecheck", "capture", "region", "--json"]
        stdout: StdioCollector { onStreamFinished: root.handleResponse("capture", this.text) }
    }
    Process {
        id: captureApplication
        command: ["surfacecheck", "capture", "application", root.applicationAddress, "--json"]
        stdout: StdioCollector { onStreamFinished: root.handleResponse("capture", this.text) }
    }
    Process {
        id: reviewAgent
        command: ["surfacecheck", "review", "latest", "--agent", "--consent-local", "--json"]
        stdout: StdioCollector { onStreamFinished: root.handleResponse("review", this.text) }
    }
    Process {
        id: exportEvidence
        command: ["surfacecheck", "export", "session-current", "--json"]
        stdout: StdioCollector { onStreamFinished: root.handleResponse("export", this.text) }
    }
    Process {
        id: handoffDefect
        command: ["surfacecheck", "handoff", "premonition", root.findingId, "--consent-external", "--json"]
        stdout: StdioCollector { onStreamFinished: root.handleResponse("handoff", this.text) }
    }
    Process {
        id: annotateEvidence
        command: ["surfacecheck", "annotate", "session-current", "--note", root.userNote, "--json"]
        stdout: StdioCollector { onStreamFinished: root.handleResponse("annotate", this.text) }
    }
    Process {
        id: selectComparison
        command: ["surfacecheck", "select-before-after", "session-current", root.beforeCaptureId, root.afterCaptureId, "--json"]
        stdout: StdioCollector { onStreamFinished: root.handleResponse("select", this.text) }
    }

    Rectangle {
        anchors.fill: parent
        color: "#171b20"
        radius: 16
        border.width: 1
        border.color: "#65727f"

        Column {
            anchors.fill: parent
            anchors.margins: 24
            spacing: 14

            Text {
                text: "SurfaceCheck"
                color: "#f4f7fa"
                font.pixelSize: 24
            }
            Text {
                text: "Capture one surface, then review the visible evidence."
                color: "#b8c2cc"
                wrapMode: Text.WordWrap
                width: parent.width
            }

            Row {
                spacing: 8
                Rectangle {
                    width: 190; height: 38; radius: 8; color: "#2d7d66"
                    Text { anchors.centerIn: parent; text: "Capture window"; color: "white" }
                    MouseArea { anchors.fill: parent; onClicked: captureWindow.running = true }
                }
                Rectangle {
                    width: 190; height: 38; radius: 8; color: "#315e93"
                    Text { anchors.centerIn: parent; text: "Select region"; color: "white" }
                    MouseArea { anchors.fill: parent; onClicked: captureRegion.running = true }
                }
            }

            Text { text: "Explicit application address (0x…):"; color: "#86b8ff"; font.pixelSize: 16 }
            TextInput {
                width: parent.width
                height: 38
                color: "#f4f7fa"
                maximumLength: 128
                text: root.applicationAddress
                onTextChanged: root.applicationAddress = text
                clip: true
            }
            Rectangle {
                width: 170; height: 34; radius: 8; color: "#3e4651"
                Text { anchors.centerIn: parent; text: "Save note"; color: "white" }
                MouseArea { anchors.fill: parent; onClicked: annotateEvidence.running = true }
            }
            Rectangle {
                width: 220; height: 38; radius: 8
                color: root.applicationAddress.length > 2 ? "#526b4b" : "#3e4651"
                Text { anchors.centerIn: parent; text: "Capture application"; color: "white" }
                MouseArea {
                    anchors.fill: parent
                    enabled: root.applicationAddress.length > 2
                    onClicked: captureApplication.running = true
                }
            }

            Text { text: "Captures"; color: "#86b8ff"; font.pixelSize: 16 }
            Text { text: root.captureSummary; color: "#e8edf2"; wrapMode: Text.WordWrap; width: parent.width }

            Text { text: "User notes"; color: "#86b8ff"; font.pixelSize: 16 }
            TextInput {
                width: parent.width
                height: 38
                color: "#f4f7fa"
                maximumLength: 16384
                text: root.userNote
                onTextChanged: root.userNote = text
                clip: true
            }

            Text { text: "Deterministic findings"; color: "#86b8ff"; font.pixelSize: 16 }
            Text { text: root.deterministicFindings; color: "#e8edf2"; wrapMode: Text.WordWrap; width: parent.width }

            Text { text: "Optional agent findings"; color: "#86b8ff"; font.pixelSize: 16 }
            Text { text: root.agentFindings; color: "#e8edf2"; wrapMode: Text.WordWrap; width: parent.width }
            Text { text: "Review sends only this local capture to the configured agent."; color: "#b8c2cc"; wrapMode: Text.WordWrap; width: parent.width }
            Rectangle {
                width: 190; height: 34; radius: 8; color: "#76552c"
                Text { anchors.centerIn: parent; text: "Review with local agent"; color: "white" }
                MouseArea { anchors.fill: parent; onClicked: reviewAgent.running = true }
            }
            TextInput {
                width: parent.width
                height: 34
                color: "#f4f7fa"
                maximumLength: 128
                text: root.findingId
                onTextChanged: root.findingId = text
                clip: true
                placeholderText: "Agent finding ID for optional handoff"
            }

            Text { text: "Before / after"; color: "#86b8ff"; font.pixelSize: 16 }
            Text { text: root.comparisonSummary; color: "#e8edf2"; wrapMode: Text.WordWrap; width: parent.width }
            Row {
                spacing: 8
                TextInput {
                    width: 200; height: 34; color: "#f4f7fa"; maximumLength: 128
                    text: root.beforeCaptureId
                    onTextChanged: root.beforeCaptureId = text
                    clip: true
                }
                TextInput {
                    width: 200; height: 34; color: "#f4f7fa"; maximumLength: 128
                    text: root.afterCaptureId
                    onTextChanged: root.afterCaptureId = text
                    clip: true
                }
            }
            Rectangle {
                width: 220; height: 34; radius: 8; color: "#3e4651"
                Text { anchors.centerIn: parent; text: "Compare before / after"; color: "white" }
                MouseArea {
                    anchors.fill: parent
                    enabled: root.beforeCaptureId.length > 0 && root.afterCaptureId.length > 0
                    onClicked: selectComparison.running = true
                }
            }

            Text { text: "Exportable evidence"; color: "#86b8ff"; font.pixelSize: 16 }
            Text { text: root.evidenceSummary; color: "#e8edf2"; wrapMode: Text.WordWrap; width: parent.width }
            Row {
                spacing: 8
                Rectangle {
                    width: 150; height: 34; radius: 8; color: "#3e4651"
                    Text { anchors.centerIn: parent; text: "Export bundle"; color: "white" }
                    MouseArea { anchors.fill: parent; onClicked: exportEvidence.running = true }
                }
                Rectangle {
                    width: 190; height: 34; radius: 8; color: "#573b66"
                    Text { anchors.centerIn: parent; text: "Send defect externally"; color: "white" }
                    MouseArea {
                        anchors.fill: parent
                        enabled: root.findingId.length > 0
                        onClicked: handoffDefect.running = true
                    }
                }
            }
        }
    }

    Keys.onEscapePressed: root.close()
}
