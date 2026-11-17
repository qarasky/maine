import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

/// Add or edit a host and its saved password. Validation text comes from the
/// same rules as every other client (maine-core).
Dialog {
    id: dlg
    modal: true
    anchors.centerIn: Overlay.overlay
    width: Math.min(460, win.width - 40)
    title: isNew ? "New host" : "Edit host"
    property bool isNew: true
    property var draft: ({})
    property string problems: ""
    property string failure: ""

    function openNew() { load(app.hostForEditor("")) }
    function openEdit(id) { load(app.hostForEditor(id)) }

    function load(json) {
        const h = JSON.parse(json)
        isNew = h.isNew === true
        draft = h
        nameField.text = h.name
        hostField.text = h.hostname
        portField.text = String(h.port)
        userField.text = h.username
        passField.text = h.password
        showPass.checked = false
        groupBox.currentIndex = 0
        for (let i = 0; i < win.snap.groups.length; ++i)
            if (win.snap.groups[i].id === h.groupId) groupBox.currentIndex = i + 1
        failure = ""
        recompute()
        open()
        nameField.forceActiveFocus()
    }

    function current() {
        const g = groupBox.currentIndex > 0 ? win.snap.groups[groupBox.currentIndex - 1].id : null
        return { id: draft.id, name: nameField.text, hostname: hostField.text, port: portField.text,
                 username: userField.text, password: passField.text, groupId: g }
    }

    function recompute() {
        // Only complain once there is something to complain about.
        problems = (nameField.text + hostField.text).length === 0 && isNew ? "" : app.validateHost(JSON.stringify(current()))
    }

    footer: DialogButtonBox {
        Button {
            text: "Save"
            highlighted: true
            enabled: dlg.problems.length === 0 && nameField.text.length > 0 && hostField.text.length > 0
            DialogButtonBox.buttonRole: DialogButtonBox.NoRole
            onClicked: {
                const failure = app.saveHost(JSON.stringify(dlg.current()))
                if (failure.length > 0) dlg.failure = failure
                else dlg.close()
            }
        }
        Button { text: "Cancel"; DialogButtonBox.buttonRole: DialogButtonBox.RejectRole }
    }

    GridLayout {
        anchors.fill: parent
        columns: 2
        columnSpacing: 12
        rowSpacing: 8

        Label { text: "Name" }
        TextField { id: nameField; Layout.fillWidth: true; onTextChanged: dlg.recompute() }
        Label { text: "Host" }
        TextField { id: hostField; Layout.fillWidth: true; placeholderText: "IP, DNS name or ssh config alias"; onTextChanged: dlg.recompute() }
        Label { text: "Port" }
        TextField { id: portField; Layout.preferredWidth: 100; inputMethodHints: Qt.ImhDigitsOnly; onTextChanged: dlg.recompute() }
        Label { text: "User" }
        TextField { id: userField; Layout.fillWidth: true; placeholderText: "optional: ssh decides"; onTextChanged: dlg.recompute() }
        Label { text: "Password" }
        RowLayout {
            Layout.fillWidth: true
            TextField {
                id: passField
                Layout.fillWidth: true
                echoMode: showPass.checked ? TextInput.Normal : TextInput.Password
                placeholderText: "optional: empty means key login"
            }
            CheckBox { id: showPass; text: "Show" }
        }
        Label { text: "Group" }
        ComboBox {
            id: groupBox
            Layout.fillWidth: true
            model: ["No group"].concat(win.snap.groups.map(g => g.name))
        }
        Label {
            Layout.columnSpan: 2
            Layout.fillWidth: true
            visible: dlg.problems.length > 0 || dlg.failure.length > 0
            wrapMode: Text.Wrap
            color: "#d05050"
            text: dlg.failure.length > 0 ? dlg.failure : dlg.problems
        }
    }
}
