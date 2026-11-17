import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

/// One-line text input in a dialog: rename a session or group, name a new group.
Dialog {
    id: dlg
    property var callback: null
    property alias value: input.text
    modal: true
    anchors.centerIn: Overlay.overlay
    width: 360
    standardButtons: Dialog.Ok | Dialog.Cancel

    function ask(heading, initial, onAccept) {
        title = heading
        input.text = initial
        callback = onAccept
        open()
        input.forceActiveFocus()
        input.selectAll()
    }

    ColumnLayout {
        anchors.fill: parent
        TextField {
            id: input
            Layout.fillWidth: true
            onAccepted: dlg.accept()
        }
    }
    onAccepted: if (callback) callback(input.text)
}
