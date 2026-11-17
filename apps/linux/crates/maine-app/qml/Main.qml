import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Maine 1.0
import app.maine.ui

ApplicationWindow {
    id: win
    width: 1180
    height: 740
    minimumWidth: 640
    minimumHeight: 420
    visible: true
    title: "Maine"
    color: pal.window

    SystemPalette { id: pal; colorGroup: SystemPalette.Active }

    /// The latest engine snapshot (see maine_engine::view::sidebar).
    property var snap: ({ state: 0, selected: null, ungrouped: [], groups: [], sessions: [], hostCount: 0, saveFailure: null })
    readonly property bool unlocked: snap.state === 2
    readonly property string selectedId: snap.selected ? snap.selected : ""
    property string banner: ""

    AppController {
        id: app
        onSidebarJsonChanged: {
            if (sidebarJson.length === 0) return
            win.snap = JSON.parse(sidebarJson)
            win.syncSessions()
        }
        onShowRequestChanged: { win.show(); win.raise(); win.requestActivate() }
        onQuitRequestChanged: { win.quitting = true; app.quit(); Qt.quit() }
        onAgentJsonChanged: { if (agentJson.length > 0) settings.agents = JSON.parse(agentJson) }
        onSyncJsonChanged: { if (syncJson.length > 0) settings.sync = JSON.parse(syncJson) }
        onOpJsonChanged: { if (opJson.length > 0) settings.handleOp(JSON.parse(opJson)) }
        onAlertJsonChanged: {
            if (alertJson.length === 0) return
            const a = JSON.parse(alertJson)
            confirmDialog.ask(a.title, a.message, "OK", function () {})
        }
        onAskpassJsonChanged: {
            if (askpassJson.length === 0) return
            askpassDialog.open(JSON.parse(askpassJson))
        }
    }

    // One ListModel row per open session. Rows are only added or removed, never
    // rebuilt, so a terminal view stays mounted for as long as its session lives.
    ListModel { id: sessionModel }

    function syncSessions() {
        const list = snap.sessions
        const ids = {}
        for (let i = 0; i < list.length; ++i) ids[list[i].id] = list[i]
        for (let i = sessionModel.count - 1; i >= 0; --i)
            if (!ids[sessionModel.get(i).sid]) sessionModel.remove(i)
        for (let i = 0; i < list.length; ++i) {
            const s = list[i]
            let found = -1
            for (let j = 0; j < sessionModel.count; ++j)
                if (sessionModel.get(j).sid === s.id) { found = j; break }
            const row = { sid: s.id, hostId: s.hostId, label: s.label, state: s.state, stateLabel: s.stateLabel,
                          live: s.live, notice: s.notice ? s.notice : "", agent: s.agent ? s.agent : "" }
            if (found < 0) sessionModel.append(row)
            else sessionModel.set(found, row)
        }
    }

    function sessionInfo(id) {
        for (let i = 0; i < snap.sessions.length; ++i) {
            const s = snap.sessions[i]
            if (s.id === id) return { id: s.id, hostId: s.hostId, label: s.label, state: s.state,
                                      stateLabel: s.stateLabel, live: s.live, notice: s.notice ? s.notice : "",
                                      agent: s.agent ? s.agent : "", agentPaused: s.agentPaused === true,
                                      handedOff: s.handedOff === true, ephemeral: s.ephemeral === true,
                                      formerAgent: s.formerAgent ? s.formerAgent : "" }
        }
        return null
    }

    function hostById(id) {
        const all = allHosts()
        for (let i = 0; i < all.length; ++i) if (all[i].id === id) return all[i]
        return null
    }

    function allHosts() {
        let all = snap.ungrouped.slice()
        for (let i = 0; i < snap.groups.length; ++i) all = all.concat(snap.groups[i].hosts)
        return all
    }

    function report(message) {
        if (message && message.length > 0) { banner = message; bannerTimer.restart() }
    }

    Timer { interval: 500; running: true; repeat: true; onTriggered: app.refresh() }
    Timer { interval: 2000; running: win.unlocked; repeat: true; onTriggered: app.tick() }
    Timer { id: bannerTimer; interval: 8000; onTriggered: win.banner = "" }

    Component.onCompleted: {
        app.start()
        if (Qt.application.arguments.indexOf("--demo") >= 0) demoLoader.active = true
    }

    // Closing hides the window and the app keeps running in the tray; without
    // a tray there is no way back, so then it quits.
    onClosing: (close) => {
        if (app.trayActive && !win.quitting) { close.accepted = false; win.hide() }
        else app.quit()
    }
    property bool quitting: false
    property string demoOp: ""
    onActiveChanged: if (active && unlocked) app.syncForeground()

    // Developer hook: `--shot=<png>` saves the window after a second and exits.
    function argValue(prefix) {
        const a = Qt.application.arguments
        for (let i = 0; i < a.length; ++i) if (a[i].indexOf(prefix) === 0) return a[i].substring(prefix.length)
        return ""
    }
    Timer {
        interval: 1800
        running: win.argValue("--shot=").length > 0
        onTriggered: { clipboard.grabWindow(win, win.argValue("--shot=")); win.quitting = true; app.quit(); Qt.quit() }
    }

    Shortcut { sequences: ["Ctrl+N"]; enabled: win.unlocked; onActivated: hostEditor.openNew() }
    Shortcut { sequences: ["Ctrl+F"]; enabled: win.unlocked; onActivated: sidebar.focusSearch() }
    Shortcut { sequences: ["Ctrl+L"]; enabled: win.unlocked; onActivated: app.lock() }
    Shortcut { sequences: ["Ctrl+Q"]; onActivated: { win.quitting = true; app.quit(); Qt.quit() } }

    Item {
        id: shotRoot
        anchors.fill: parent

        RowLayout {
            anchors.fill: parent
            spacing: 0
            visible: win.unlocked

            Sidebar {
                id: sidebar
                Layout.preferredWidth: 280
                Layout.minimumWidth: 200
                Layout.maximumWidth: 480
                Layout.fillHeight: true
            }
            Rectangle { Layout.fillHeight: true; width: 1; color: Qt.darker(pal.window, 1.25) }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 0

                SessionHeader {
                    Layout.fillWidth: true
                    info: win.sessionInfo(win.selectedId)
                    visible: info !== null
                }

                Rectangle {
                    id: stage
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    color: win.selectedId === "" ? pal.window : "#1e1e1e"

                    EmptyState {
                        anchors.centerIn: parent
                        visible: win.selectedId === ""
                        hostCount: win.snap.hostCount
                    }

                    // Every open session keeps its own terminal view, all the same
                    // size, stacked; only the selected one is shown.
                    Repeater {
                        model: sessionModel
                        delegate: TerminalItem {
                            required property string sid
                        required property string agent
                            anchors.fill: parent
                            anchors.margins: 6
                            sessionId: sid
                        userDriven: agent === ""
                            visible: win.unlocked && sid === win.selectedId
                            onVisibleChanged: if (visible) forceActiveFocus()
                            Component.onCompleted: if (visible) forceActiveFocus()
                        }
                    }
                }
            }
        }

        UnlockGate {
            anchors.fill: parent
            visible: !win.unlocked
            mode: win.snap.state
        }

        // Transient message (save failures, errors from actions).
        Rectangle {
            id: bannerBox
            visible: win.banner.length > 0 || (win.snap.saveFailure !== null && win.snap.saveFailure !== undefined)
            anchors { left: parent.left; right: parent.right; bottom: parent.bottom; margins: 12 }
            height: bannerText.implicitHeight + 16
            radius: 6
            color: "#7a2e2e"
            z: 50
            Label {
                id: bannerText
                anchors { fill: parent; margins: 8 }
                text: win.banner.length > 0 ? win.banner : (win.snap.saveFailure ? win.snap.saveFailure : "")
                color: "white"
                wrapMode: Text.Wrap
            }
        }
    }

    ClipboardHelper { id: clipboard }
    HostEditor { id: hostEditor }
    SettingsDialog { id: settings }
    TextPrompt { id: textPrompt }
    ImportDialog { id: importDialog }
    ConfirmDialog { id: confirmDialog }
    AskpassDialog { id: askpassDialog }

    Loader {
        id: demoLoader
        active: false
        source: "DemoDriver.qml"
    }
}
