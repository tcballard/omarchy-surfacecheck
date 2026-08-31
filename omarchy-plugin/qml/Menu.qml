import QtQuick
import Quickshell
import Quickshell.Io

Item {
    id: root
    visible: false
    width: 260
    height: 68
    focus: true

    function open(payloadJson) {
        visible = true
        forceActiveFocus()
    }

    function close() {
        visible = false
    }

    Process {
        id: summonPanel
        command: ["omarchy-shell", "shell", "summon", "tcballard.surfacecheck", "{}"]
    }

    Rectangle {
        anchors.fill: parent
        color: "#20252b"
        radius: 12
        border.width: 1
        border.color: "#6d7885"

        Text {
            anchors.centerIn: parent
            text: "Check this surface"
            color: "#f4f7fa"
            font.pixelSize: 15
        }

        MouseArea {
            anchors.fill: parent
            onClicked: {
                summonPanel.running = true
                root.close()
            }
        }
    }

    Keys.onReturnPressed: {
        summonPanel.running = true
        root.close()
    }
    Keys.onEscapePressed: root.close()
}
