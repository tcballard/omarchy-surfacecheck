import QtQuick
import Quickshell
import Quickshell.Io

Item {
    id: root
    visible: false
    anchors.fill: parent
    focus: true

    function open(payloadJson) {
        visible = true
        forceActiveFocus()
    }

    function close() {
        visible = false
    }

    Process { id: captureWindow; command: ["surfacecheck", "capture", "window", "--json"] }
    Process { id: captureRegion; command: ["surfacecheck", "capture", "region", "--json"] }

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
        captureRegion.start()
        root.close()
    }
    Keys.onWPressed: {
        captureWindow.start()
        root.close()
    }
    Keys.onEscapePressed: root.close()
}
