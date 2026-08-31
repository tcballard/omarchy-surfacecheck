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

    Process { id: captureRegion; command: ["surfacecheck", "capture", "region", "--json"] }

    Rectangle {
        anchors.fill: parent
        color: "#66101820"
        border.width: 2
        border.color: "#86b8ff"
        Text {
            anchors.centerIn: parent
            text: "Select a region with the pointer, or press Enter. Esc cancels."
            color: "white"
            font.pixelSize: 20
        }
    }

    Keys.onReturnPressed: {
        captureRegion.start()
        root.close()
    }
    Keys.onEscapePressed: root.close()
}
