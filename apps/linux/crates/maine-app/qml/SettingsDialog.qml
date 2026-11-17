import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

/// Settings: Sync today; Agents joins it when the MCP server lands in the UI.
Dialog {
    id: dlg
    modal: true
    anchors.centerIn: Overlay.overlay
    width: Math.min(640, win.width - 40)
    height: Math.min(560, win.height - 40)
    title: "Settings"
    standardButtons: Dialog.Close

    property var sync: ({ ready: false, profiles: [], activeId: null, status: "off", statusLabel: "Sync off" })
    property var agents: ({ ready: false, enabled: false, active: 0, error: null, clients: [], groups: [], hosts: [] })
    property var pairing: ({ name: "", token: "", claudeCodeCommand: "", desktopJson: "" })
    property var logEntries: []
    property string busy: ""        // the sync operation in flight, if any
    property string message: ""     // last operation error
    property string shownToken: ""  // a token to show exactly once
    property string pendingServer: ""
    property string pendingToken: ""
    property string pendingName: ""

    function openSettings(tab) { tabs.currentIndex = tab; message = ""; open() }

    function handleOp(op) {
        if (op.op === busy) busy = ""
        if (!op.ok) { message = op.error; return }
        message = ""
        if (op.op === "agentPair") {
            pairing = op.data
            pairDialog.open()
            return
        }
        if (op.op === "create" || op.op === "rotate") {
            shownToken = op.data.token
            tokenDialog.open()
        } else if (op.op === "join" && op.data.emptySlot) {
            confirmDialog.ask("Empty slot", "This slot has no vault yet. Upload this device's vault to it?", "Upload",
                              function () { busy = "pushEmpty"; app.syncPushEmpty(pendingServer, pendingToken, pendingName) })
        }
    }

    function saveAccess(id, isGroup, on) {
        const groups = dlg.agents.groups.slice()
        const hosts = dlg.agents.hosts.slice()
        const list = isGroup ? groups : hosts
        const at = list.indexOf(id)
        if (on && at < 0) list.push(id)
        if (!on && at >= 0) list.splice(at, 1)
        win.report(app.agentSetAccess(JSON.stringify({ groups: groups, hosts: hosts })))
    }

    function dotColor(kind) {
        switch (kind) {
        case "idle": return "#4caf50"
        case "syncing": return "#e0a030"
        case "off": return "#888888"
        default: return "#d05050"
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 8

        TabBar {
            id: tabs
            Layout.fillWidth: true
            TabButton { text: "Sync" }
            TabButton { text: "Agents" }
        }

        StackLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: tabs.currentIndex

            // ---- Sync ----
            ColumnLayout {
                spacing: 10

                RowLayout {
                    spacing: 8
                    Rectangle { width: 10; height: 10; radius: 5; color: dlg.dotColor(dlg.sync.status) }
                    Label { text: dlg.sync.statusLabel; font.weight: Font.DemiBold; Layout.fillWidth: true; elide: Text.ElideRight }
                    BusyIndicator { visible: dlg.busy !== ""; running: visible; implicitWidth: 22; implicitHeight: 22 }
                }
                Label {
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    font.pixelSize: 12
                    opacity: 0.7
                    text: "Sync is optional and self-hosted. Your vault is uploaded encrypted; the server never sees your master password. Tokens are kept in the system keyring."
                }
                Label {
                    visible: dlg.message.length > 0
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    color: "#d05050"
                    text: dlg.message
                }

                ListView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    model: dlg.sync.profiles
                    spacing: 6
                    delegate: Rectangle {
                        required property var modelData
                        width: ListView.view.width
                        implicitHeight: pcol.implicitHeight + 16
                        radius: 6
                        color: Qt.rgba(pal.windowText.r, pal.windowText.g, pal.windowText.b, 0.06)
                        border.width: modelData.id === dlg.sync.activeId ? 1 : 0
                        border.color: pal.highlight
                        ColumnLayout {
                            id: pcol
                            anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter; margins: 10 }
                            spacing: 4
                            RowLayout {
                                Label { text: modelData.name; font.weight: Font.DemiBold }
                                Label { visible: modelData.id === dlg.sync.activeId; text: "active"; color: pal.highlight; font.pixelSize: 11 }
                                Item { Layout.fillWidth: true }
                                Label {
                                    font.pixelSize: 11; opacity: 0.6
                                    text: modelData.lastSyncedAt ? "synced " + new Date(modelData.lastSyncedAt * 1000).toLocaleString(Qt.locale(), Locale.ShortFormat) : "never synced"
                                }
                            }
                            Label { text: modelData.serverURL; font.pixelSize: 11; opacity: 0.65; elide: Text.ElideRight; Layout.fillWidth: true }
                            Flow {
                                Layout.fillWidth: true
                                spacing: 6
                                Button {
                                    visible: modelData.id !== dlg.sync.activeId
                                    text: "Make active"
                                    onClicked: confirmDialog.ask("Switch workspace", "Switching replaces the vault on this device with this slot's vault. Open sessions are saved, closed and restored when you come back.", "Switch",
                                                                 function () { dlg.busy = "activate"; app.syncActivate(modelData.id) })
                                }
                                Button { text: "Rename"; onClicked: textPrompt.ask("Rename sync profile", modelData.name, function (v) { dlg.message = app.syncRename(modelData.id, v) }) }
                                Button { text: "New token"; onClicked: confirmDialog.ask("Rotate token", "The old token stops working on every device. Continue?", "Rotate", function () { dlg.busy = "rotate"; app.syncRotate(modelData.id) }) }
                                Button { text: "Stop syncing"; onClicked: confirmDialog.ask("Stop syncing", "This device stops syncing; the vault stays here and the server's copy is untouched.", "Stop", function () { dlg.busy = "disable"; app.syncDisable(modelData.id) }) }
                                Button { text: "Delete server data"; onClicked: confirmDialog.ask("Delete server data", "Deletes the vault copy and history on the server. Other devices keep their local copy.", "Delete", function () { dlg.busy = "deleteRemote"; app.syncDeleteRemote(modelData.id) }) }
                            }
                        }
                    }
                }
                Label {
                    visible: dlg.sync.profiles.length === 0
                    text: "Not syncing. Create a slot on your server or join one with a token."
                    opacity: 0.6
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                }
                RowLayout {
                    Button { text: "Create sync slot…"; enabled: dlg.busy === ""; onClicked: createDialog.openIt() }
                    Button { text: "Join with token…"; enabled: dlg.busy === ""; onClicked: joinDialog.openIt() }
                }
            }

            // ---- Agents ----
            ColumnLayout {
                spacing: 10

                Switch {
                    text: "Allow AI agents (MCP) to use my hosts"
                    checked: dlg.agents.enabled === true
                    onToggled: app.agentSetEnabled(checked)
                }
                Label {
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                    font.pixelSize: 12
                    opacity: 0.7
                    text: "Off by default. Agents connect through `maine --mcp`, see only the hosts you allow below, work in sessions you can watch and take over, and never get to read a saved password. Rules stay on this device and are never synced."
                }
                Label { visible: dlg.agents.error !== null && dlg.agents.error !== undefined; color: "#d05050"; Layout.fillWidth: true; wrapMode: Text.Wrap; text: dlg.agents.error ? dlg.agents.error : "" }

                Label { text: "Paired clients"; font.weight: Font.DemiBold }
                Repeater {
                    model: dlg.agents.clients
                    delegate: RowLayout {
                        required property var modelData
                        Layout.fillWidth: true
                        Label { text: modelData.name; Layout.fillWidth: true; elide: Text.ElideRight }
                        Button { text: "Revoke"; onClicked: app.agentRevoke(modelData.id) }
                    }
                }
                Label { visible: dlg.agents.clients.length === 0; text: "No clients paired yet."; opacity: 0.6 }
                Button { text: "Pair a client…"; enabled: dlg.agents.enabled === true; onClicked: textPrompt.ask("Name this client", "Claude Code", function (v) { app.agentAddClient(v) }) }

                Label { text: "Allowed hosts"; font.weight: Font.DemiBold; Layout.topMargin: 6 }
                ScrollView {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    ColumnLayout {
                        width: parent.width
                        spacing: 0
                        Repeater {
                            model: win.snap.groups
                            delegate: CheckBox {
                                required property var modelData
                                text: "Group: " + modelData.name + " (" + modelData.hosts.length + ")"
                                checked: dlg.agents.groups.indexOf(modelData.id) >= 0
                                onToggled: dlg.saveAccess(modelData.id, true, checked)
                            }
                        }
                        Repeater {
                            model: win.allHosts()
                            delegate: CheckBox {
                                required property var modelData
                                text: modelData.name + "  ·  " + modelData.subtitle
                                checked: dlg.agents.hosts.indexOf(modelData.id) >= 0
                                onToggled: dlg.saveAccess(modelData.id, false, checked)
                            }
                        }
                    }
                }
                RowLayout {
                    Button { text: "Stop all agent sessions"; enabled: dlg.agents.active > 0; onClicked: app.agentStopAll() }
                    Button { text: "Activity log…"; onClicked: { dlg.logEntries = JSON.parse(app.agentLog()); logDialog.open() } }
                }
            }
        }
    }

    // ---- create ----
    Dialog {
        id: createDialog
        modal: true
        anchors.centerIn: Overlay.overlay
        width: Math.min(460, win.width - 40)
        title: "Create sync slot"
        standardButtons: Dialog.Ok | Dialog.Cancel
        function openIt() { serverField.text = "http://"; nameField.text = "My server"; secretField.text = ""; open() }
        onAccepted: { dlg.busy = "create"; app.syncCreate(serverField.text, nameField.text, secretField.text) }
        GridLayout {
            anchors.fill: parent
            columns: 2
            Label { text: "Server" }
            TextField { id: serverField; Layout.fillWidth: true; placeholderText: "https://sync.example.com" }
            Label { text: "Name" }
            TextField { id: nameField; Layout.fillWidth: true }
            Label { text: "Create secret" }
            TextField { id: secretField; Layout.fillWidth: true; echoMode: TextInput.Password; placeholderText: "only if the server requires one" }
        }
    }

    // ---- join ----
    Dialog {
        id: joinDialog
        modal: true
        anchors.centerIn: Overlay.overlay
        width: Math.min(460, win.width - 40)
        title: "Join a sync slot"
        standardButtons: Dialog.Ok | Dialog.Cancel
        function openIt() { jServer.text = "http://"; jToken.text = ""; jName.text = "My server"; jMaster.text = ""; open() }
        onAccepted: {
            dlg.pendingServer = jServer.text; dlg.pendingToken = jToken.text; dlg.pendingName = jName.text
            dlg.busy = "join"
            app.syncJoin(jServer.text, jToken.text, jName.text, jMaster.text)
        }
        GridLayout {
            anchors.fill: parent
            columns: 2
            Label { text: "Server" }
            TextField { id: jServer; Layout.fillWidth: true }
            Label { text: "Token" }
            TextField { id: jToken; Layout.fillWidth: true; placeholderText: "ABCD-EFGH-…" }
            Label { text: "Name" }
            TextField { id: jName; Layout.fillWidth: true }
            Label { text: "Master password" }
            TextField { id: jMaster; Layout.fillWidth: true; echoMode: TextInput.Password; placeholderText: "of the vault on the server" }
            Label {
                Layout.columnSpan: 2
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                font.pixelSize: 12
                opacity: 0.7
                text: "The server's vault replaces the one on this device (yours is kept as a copy next to it)."
            }
        }
    }

    // ---- token shown once ----
    Dialog {
        id: tokenDialog
        modal: true
        anchors.centerIn: Overlay.overlay
        width: Math.min(500, win.width - 40)
        title: "Your sync token"
        standardButtons: Dialog.Close
        ColumnLayout {
            anchors.fill: parent
            spacing: 10
            Label { Layout.fillWidth: true; wrapMode: Text.Wrap; text: "Shown only now. Copy it to join from another device; it cannot be shown again." }
            TextField { Layout.fillWidth: true; readOnly: true; text: dlg.shownToken; font.family: "monospace"; selectByMouse: true }
            Button { text: "Copy token"; onClicked: clipboard.copy(dlg.shownToken) }
        }
        onClosed: dlg.shownToken = ""
    }

    // ---- pairing result (token shown once) ----
    Dialog {
        id: pairDialog
        modal: true
        anchors.centerIn: Overlay.overlay
        width: Math.min(620, win.width - 40)
        title: "Pair " + dlg.pairing.name
        standardButtons: Dialog.Close
        ColumnLayout {
            anchors.fill: parent
            spacing: 8
            Label { Layout.fillWidth: true; wrapMode: Text.Wrap; text: "The token is shown only now. Run one of these where your MCP client lives." }
            Label { text: "Claude Code"; font.weight: Font.DemiBold }
            TextArea { Layout.fillWidth: true; readOnly: true; wrapMode: Text.WrapAnywhere; text: dlg.pairing.claudeCodeCommand; font.family: "monospace"; selectByMouse: true }
            Button { text: "Copy command"; onClicked: clipboard.copy(dlg.pairing.claudeCodeCommand) }
            Label { text: "Claude Desktop / other clients (JSON)"; font.weight: Font.DemiBold }
            TextArea { Layout.fillWidth: true; Layout.preferredHeight: 110; readOnly: true; text: dlg.pairing.desktopJson; font.family: "monospace"; selectByMouse: true }
            Button { text: "Copy JSON"; onClicked: clipboard.copy(dlg.pairing.desktopJson) }
        }
        onClosed: dlg.pairing = ({ name: "", token: "", claudeCodeCommand: "", desktopJson: "" })
    }

    // ---- activity log ----
    Dialog {
        id: logDialog
        modal: true
        anchors.centerIn: Overlay.overlay
        width: Math.min(700, win.width - 40)
        height: Math.min(480, win.height - 40)
        title: "Agent activity on this device"
        standardButtons: Dialog.Close
        ColumnLayout {
            anchors.fill: parent
            Label { visible: dlg.logEntries.length === 0; text: "Nothing yet."; opacity: 0.6 }
            ListView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: dlg.logEntries
                delegate: Label {
                    required property var modelData
                    width: ListView.view.width
                    wrapMode: Text.WrapAnywhere
                    font.pixelSize: 12
                    text: new Date((modelData.at + 978307200) * 1000).toLocaleString(Qt.locale(), Locale.ShortFormat) + "  " + modelData.client + " · " + modelData.hostName
                          + " · " + modelData.kind + (modelData.detail ? "  " + modelData.detail : "") + (modelData.exitCode !== undefined ? "  (exit " + modelData.exitCode + ")" : "")
                }
            }
            Button { text: "Clear log"; enabled: dlg.logEntries.length > 0; onClicked: { app.agentClearLog(); dlg.logEntries = [] } }
        }
    }
}
