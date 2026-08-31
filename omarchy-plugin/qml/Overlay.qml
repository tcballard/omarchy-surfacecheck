import QtQuick
import Quickshell
import Quickshell.Io

Item {
    id: root
    visible: false
    anchors.fill: parent
    focus: true
    property string capturePayload: "{}"

    function open(payloadJson) {
        visible = true
        forceActiveFocus()
    }

    function close() {
        visible = false
    }

    function forwardCapture(rawText) {
        capturePayload = String(rawText || "{}").slice(0, 65536)
        summonPanel.running = true
    }

    Process {
        id: captureWindow
        command: ["surfacecheck", "capture", "window", "--json"]
        stdout: StdioCollector { onStreamFinished: root.forwardCapture(this.text) }
    }
    Process {
        id: captureRegion
        command: ["surfacecheck", "capture", "region", "--json"]
        stdout: StdioCollector { onStreamFinished: root.forwardCapture(this.text) }
    }
    Process {
        id: summonPanel
        command: ["omarchy-shell", "shell", "summon", "tcballard.surfacecheck", root.capturePayload]
    }

    Rectangle {
        anchors.fill: parent
        color: "#66101820"
        border.width: 2
        border.color: "#86b8ff"
        Column {
            anchors.centerIn: parent
            spacing: 12
            Text { text: "Check this surface"; color: "white"; font.pixelSize: 22 }
            Text { text: "Enter: region  •  W: active window  •  Esc: cancel"; color: "#d8e5f2" }
        }
    }

    Keys.onReturnPressed: {
        captureRegion.running = true
        root.close()
    }
    Keys.onWPressed: {
        captureWindow.running = true
        root.close()
    }
    Keys.onEscapePressed: root.close()
}
