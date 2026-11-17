import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

/// Master-password gate, shown on every launch and whenever the vault is locked.
Rectangle {
    id: gate
    /// 0 = no vault yet (create), 1 = locked (unlock)
    property int mode: 1
    color: pal.window
    z: 100

    property string message: ""
    readonly property bool creating: mode === 0

    function submit() {
        message = ""
        if (creating && field.text !== confirm.text) { message = "The two passwords do not match."; return }
        const failure = creating ? app.createVault(field.text) : app.unlock(field.text)
        if (failure.length > 0) message = failure
        else { field.text = ""; confirm.text = "" }
    }

    onVisibleChanged: if (visible) { field.text = ""; confirm.text = ""; message = ""; field.forceActiveFocus() }

    ColumnLayout {
        anchors.centerIn: parent
        width: Math.min(360, parent.width - 48)
        spacing: 12

        Label {
            text: "Maine"
            font.pixelSize: 34
            font.weight: Font.DemiBold
            Layout.alignment: Qt.AlignHCenter
        }
        Label {
            text: gate.creating ? "Create a master password for your vault" : "Enter your master password"
            Layout.alignment: Qt.AlignHCenter
            color: Qt.rgba(pal.windowText.r, pal.windowText.g, pal.windowText.b, 0.7)
        }
        TextField {
            id: field
            Layout.fillWidth: true
            echoMode: TextInput.Password
            placeholderText: "Master password"
            focus: true
            onAccepted: gate.creating ? confirm.forceActiveFocus() : gate.submit()
        }
        TextField {
            id: confirm
            Layout.fillWidth: true
            visible: gate.creating
            echoMode: TextInput.Password
            placeholderText: "Repeat master password"
            onAccepted: gate.submit()
        }
        Label {
            visible: gate.creating
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            font.pixelSize: 12
            color: Qt.rgba(pal.windowText.r, pal.windowText.g, pal.windowText.b, 0.6)
            text: "At least 12 characters. It is never stored; there is no way to recover it."
        }
        Label {
            visible: gate.message.length > 0
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            color: "#d05050"
            text: gate.message
        }
        Button {
            Layout.fillWidth: true
            text: gate.creating ? "Create vault" : "Unlock"
            highlighted: true
            onClicked: gate.submit()
        }
    }
}
