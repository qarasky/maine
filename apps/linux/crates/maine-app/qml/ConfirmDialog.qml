import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: dlg
    property var callback: null
    property string message: ""
    property string actionText: "OK"
    modal: true
    anchors.centerIn: Overlay.overlay
    width: 380
    standardButtons: Dialog.Cancel

    function ask(heading, text, action, onAccept) {
        title = heading
        message = text
        actionText = action
        callback = onAccept
        open()
    }

    footer: DialogButtonBox {
        Button { text: dlg.actionText; highlighted: true; DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole }
        Button { text: "Cancel"; DialogButtonBox.buttonRole: DialogButtonBox.RejectRole }
    }

    Label { width: parent.width; wrapMode: Text.Wrap; text: dlg.message }
    onAccepted: if (callback) callback()
}
