import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

/// Title row above the terminal: name, endpoint, state, and the actions that apply.
Rectangle {
    id: header
    property var info: null
    readonly property var host: info ? win.hostById(info.hostId) : null
    implicitHeight: col.implicitHeight + 16
    color: Qt.darker(pal.window, 1.04)

    function dotColor(state) {
        switch (state) {
        case "connected": return "#4caf50"
        case "connecting": return "#e0a030"
        case "failed": return "#d05050"
        case "closed": return info && info.notice ? "#d05050" : "#888888"
        default: return "#888888"
        }
    }

    ColumnLayout {
        id: col
        anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter; margins: 12 }
        spacing: 6

        RowLayout {
            Layout.fillWidth: true
            spacing: 10
            Rectangle { width: 10; height: 10; radius: 5; color: header.info ? header.dotColor(header.info.state) : "#888" }
            Label {
                text: header.host && header.info ? header.host.name + "  ·  " + header.info.label : ""
                font.weight: Font.DemiBold
                elide: Text.ElideRight
            }
            Label {
                id: endpoint
                text: header.host ? header.host.subtitle : ""
                color: Qt.rgba(pal.windowText.r, pal.windowText.g, pal.windowText.b, 0.65)
                elide: Text.ElideRight
                Layout.fillWidth: true
                // Selectable endpoint text, like the macOS header.
                TextEdit { visible: false }
            }
            Label {
                text: header.info ? header.info.stateLabel : ""
                color: Qt.rgba(pal.windowText.r, pal.windowText.g, pal.windowText.b, 0.65)
            }
            Button {
                visible: header.info && !header.info.live
                text: "Reconnect"
                onClicked: app.reconnectSession(header.info.id)
            }
            Button {
                visible: header.info && header.info.live
                text: "Disconnect"
                onClicked: app.disconnectSession(header.info.id)
            }
            Button {
                text: "Close"
                onClicked: win.report(app.closeSession(header.info.id))
            }
        }
        // An agent is driving, or drove, this session: visible and interruptible.
        Rectangle {
            visible: header.info && (header.info.agent.length > 0 || header.info.formerAgent.length > 0)
            Layout.fillWidth: true
            implicitHeight: agentRow.implicitHeight + 12
            radius: 5
            color: header.info && header.info.agent.length > 0 ? "#2d4a5a" : "#4a4a2d"
            RowLayout {
                id: agentRow
                anchors { fill: parent; margins: 6 }
                spacing: 8
                Label {
                    Layout.fillWidth: true
                    color: "white"
                    elide: Text.ElideRight
                    text: !header.info ? "" : header.info.agent.length > 0
                          ? (header.info.agentPaused ? header.info.agent + " is paused on this session." : header.info.agent + " is driving this session.")
                          : header.info.formerAgent + " is disconnected. Keep this session or close it."
                }
                Button {
                    visible: header.info && header.info.agent.length > 0
                    text: header.info && header.info.agentPaused ? "Resume" : "Pause"
                    onClicked: app.agentPause(header.info.id, !header.info.agentPaused)
                }
                Button { visible: header.info && header.info.agent.length > 0; text: "Stop"; onClicked: app.agentStop(header.info.id) }
                Button { visible: header.info && header.info.ephemeral; text: "Keep"; onClicked: app.agentKeep(header.info.id) }
            }
        }
        Rectangle {
            visible: header.info && header.info.notice.length > 0
            Layout.fillWidth: true
            implicitHeight: notice.implicitHeight + 12
            radius: 5
            color: "#5a2d2d"
            Label {
                id: notice
                anchors { fill: parent; margins: 6 }
                text: header.info ? header.info.notice : ""
                color: "white"
                wrapMode: Text.Wrap
            }
        }
    }
}
