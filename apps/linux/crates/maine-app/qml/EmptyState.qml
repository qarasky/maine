import QtQuick
import QtQuick.Controls

Column {
    id: root
    property int hostCount: 0
    spacing: 8
    Label {
        anchors.horizontalCenter: parent.horizontalCenter
        text: root.hostCount === 0 ? "No hosts yet" : "No session open"
        font.pixelSize: 20
        font.weight: Font.DemiBold
    }
    Label {
        anchors.horizontalCenter: parent.horizontalCenter
        text: root.hostCount === 0 ? "Add a host with the + button (Ctrl+N)." : "Pick a host in the sidebar to connect."
        color: Qt.rgba(pal.windowText.r, pal.windowText.g, pal.windowText.b, 0.65)
    }
}
