import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

/// ssh asked for something Maine will not answer from the vault: a key
/// passphrase, an MFA code, or trust for a new host key.
Dialog {
    id: dlg
    modal: true
    anchors.centerIn: Overlay.overlay
    width: Math.min(460, win.width - 40)
    title: "SSH authentication"
    closePolicy: Popup.NoAutoClose
    property var request: ({ id: 0, host: "", prompt: "", confirmation: false })
    property bool answered: false

    function open(req) {
        request = req
        answered = false
        input.text = ""
        visible = true
        if (!req.confirmation) input.forceActiveFocus()
    }

    footer: DialogButtonBox {
        Button {
            text: dlg.request.confirmation ? "Trust host" : "Continue"
            highlighted: true
            DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
        }
        Button { text: "Cancel"; DialogButtonBox.buttonRole: DialogButtonBox.RejectRole }
    }

    onAccepted: { answered = true; app.answerAskpass(request.id, request.confirmation ? "yes" : input.text, true) }
    // Closing any other way is a cancel, so ssh never waits forever.
    onClosed: { if (!answered) app.answerAskpass(request.id, "", false); input.text = "" }

    contentItem: ColumnLayout {
        spacing: 10
        Label {
            visible: dlg.request.host.length > 0
            text: dlg.request.host
            font.weight: Font.DemiBold
        }
        Label { Layout.fillWidth: true; wrapMode: Text.Wrap; text: dlg.request.prompt }
        TextField {
            id: input
            visible: !dlg.request.confirmation
            Layout.fillWidth: true
            echoMode: TextInput.Password
            onAccepted: dlg.accept()
        }
    }
}
