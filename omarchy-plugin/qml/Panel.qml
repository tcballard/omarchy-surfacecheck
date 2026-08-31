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
    }

    function close() {
        visible = false
    }

    Process { id: captureWindow; command: ["surfacecheck", "capture", "window", "--json"] }
    Process { id: captureRegion; command: ["surfacecheck", "capture", "region", "--json"] }
    Process { id: captureApplication; command: ["surfacecheck", "capture", "application", root.applicationAddress, "--json"] }
    Process { id: reviewAgent; command: ["surfacecheck", "review", "latest", "--agent", "--consent-local", "--json"] }
    Process { id: exportEvidence; command: ["surfacecheck", "export", "session-current", "--json"] }
    Process { id: handoffDefect; command: ["surfacecheck", "handoff", "premonition", root.findingId, "--consent-external", "--json"] }
    Process { id: annotateEvidence; command: ["surfacecheck", "annotate", "session-current", "--note", root.userNote, "--json"] }
    Process { id: selectComparison; command: ["surfacecheck", "select-before-after", "session-current", root.beforeCaptureId, root.afterCaptureId, "--json"] }

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
                    MouseArea { anchors.fill: parent; onClicked: captureWindow.start() }
                }
                Rectangle {
                    width: 190; height: 38; radius: 8; color: "#315e93"
                    Text { anchors.centerIn: parent; text: "Select region"; color: "white" }
                    MouseArea { anchors.fill: parent; onClicked: captureRegion.start() }
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
                MouseArea { anchors.fill: parent; onClicked: annotateEvidence.start() }
            }
            Rectangle {
                width: 220; height: 38; radius: 8
                color: root.applicationAddress.length > 2 ? "#526b4b" : "#3e4651"
                Text { anchors.centerIn: parent; text: "Capture application"; color: "white" }
                MouseArea {
                    anchors.fill: parent
                    enabled: root.applicationAddress.length > 2
                    onClicked: captureApplication.start()
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
                MouseArea { anchors.fill: parent; onClicked: reviewAgent.start() }
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
                    onClicked: selectComparison.start()
                }
            }

            Text { text: "Exportable evidence"; color: "#86b8ff"; font.pixelSize: 16 }
            Text { text: root.evidenceSummary; color: "#e8edf2"; wrapMode: Text.WordWrap; width: parent.width }
            Row {
                spacing: 8
                Rectangle {
                    width: 150; height: 34; radius: 8; color: "#3e4651"
                    Text { anchors.centerIn: parent; text: "Export bundle"; color: "white" }
                    MouseArea { anchors.fill: parent; onClicked: exportEvidence.start() }
                }
                Rectangle {
                    width: 190; height: 34; radius: 8; color: "#573b66"
                    Text { anchors.centerIn: parent; text: "Send defect externally"; color: "white" }
                    MouseArea {
                        anchors.fill: parent
                        enabled: root.findingId.length > 0
                        onClicked: handoffDefect.start()
                    }
                }
            }
        }
    }

    Keys.onEscapePressed: root.close()
}
