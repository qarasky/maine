import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

/// Sidebar-only navigation: search, ungrouped hosts, collapsible groups, and
/// each host's open sessions nested under it. Whole rows are hit targets.
Rectangle {
    id: root
    color: Qt.darker(pal.window, 1.04)

    property string query: ""
    property var ctxHost: null
    property var ctxSession: null
    property var ctxGroup: null

    function focusSearch() { search.forceActiveFocus(); search.selectAll() }

    function matches(h, needle) {
        return needle === "" || h.name.toLowerCase().indexOf(needle) >= 0
            || h.hostname.toLowerCase().indexOf(needle) >= 0 || h.username.toLowerCase().indexOf(needle) >= 0
    }

    function buildRows(snap, query) {
        const out = []
        const needle = query.trim().toLowerCase()
        function pushHost(h, indent, groupId) {
            out.push({ kind: "host", host: h, indent: indent, groupId: groupId })
            for (let i = 0; i < h.sessions.length; ++i)
                out.push({ kind: "session", session: h.sessions[i], host: h, indent: indent + 1 })
        }
        for (let i = 0; i < snap.ungrouped.length; ++i)
            if (matches(snap.ungrouped[i], needle)) pushHost(snap.ungrouped[i], 0, "")
        for (let g = 0; g < snap.groups.length; ++g) {
            const group = snap.groups[g]
            const shown = group.hosts.filter(h => matches(h, needle))
            if (needle !== "" && shown.length === 0) continue
            out.push({ kind: "group", group: group })
            if (!group.collapsed || needle !== "")
                for (let i = 0; i < shown.length; ++i) pushHost(shown[i], 1, group.id)
        }
        return out
    }

    readonly property var rows: buildRows(win.snap, query)

    function dropOn(payload, target) {
        if (payload.indexOf("host:") === 0) {
            const id = payload.substring(5)
            if (target.kind === "host") app.moveHost(id, target.groupId, target.host.id)
            else if (target.kind === "group") app.moveHost(id, target.group.id, "")
            else app.moveHost(id, "", "")
        } else if (payload.indexOf("group:") === 0) {
            const id = payload.substring(6)
            app.moveGroup(id, target.kind === "group" ? target.group.id : "")
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        TextField {
            id: search
            Layout.fillWidth: true
            Layout.margins: 8
            placeholderText: "Search hosts"
            text: root.query
            onTextChanged: root.query = text
            Keys.onEscapePressed: { text = ""; focus = false }
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            // Dropping into empty space ungroups the host / moves the group last.
            DropArea {
                anchors.fill: parent
                keys: ["text/plain"]
                onDropped: (drop) => root.dropOn(drop.text, { kind: "end" })
            }
            Label {
                anchors.centerIn: parent
                visible: win.snap.hostCount === 0
                text: "No hosts"
                color: Qt.rgba(pal.windowText.r, pal.windowText.g, pal.windowText.b, 0.5)
            }

        ListView {
            id: list
            anchors.fill: parent
            clip: true
            model: root.rows
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: ScrollBar {}

            delegate: Item {
                id: row
                required property var modelData
                readonly property string kind: modelData.kind
                width: ListView.view.width
                height: kind === "host" ? 44 : 30

                readonly property bool isSelected: kind === "host" ? modelData.host.sessions.some(s => s.id === win.selectedId)
                                                 : kind === "session" ? modelData.session.id === win.selectedId : false

                Rectangle {
                    anchors { fill: parent; leftMargin: 6; rightMargin: 6; topMargin: 1; bottomMargin: 1 }
                    radius: 6
                    color: row.isSelected ? pal.highlight : (hover.hovered || drop.containsDrag
                           ? Qt.rgba(pal.windowText.r, pal.windowText.g, pal.windowText.b, 0.08) : "transparent")
                    opacity: row.isSelected && row.kind === "host" ? 0.35 : 1
                }

                HoverHandler { id: hover }

                // --- drag and drop (host rows and group headers) ---
                Drag.dragType: Drag.Automatic
                Drag.supportedActions: Qt.MoveAction
                Drag.mimeData: ({ "text/plain": row.kind === "group" ? "group:" + modelData.group.id : "host:" + (modelData.host ? modelData.host.id : "") })
                DragHandler {
                    id: drag
                    enabled: row.kind !== "session"
                    target: null
                    onActiveChanged: {
                        if (active) row.grabToImage(function(result) { row.Drag.imageSource = result.url; row.Drag.active = true })
                        else row.Drag.active = false
                    }
                }
                DropArea {
                    id: drop
                    anchors.fill: parent
                    enabled: row.kind !== "session"
                    keys: ["text/plain"]
                    onDropped: (d) => root.dropOn(d.text, row.modelData)
                }

                // --- group header ---
                RowLayout {
                    visible: row.kind === "group"
                    anchors { fill: parent; leftMargin: 14; rightMargin: 12 }
                    spacing: 6
                    Label { text: row.kind === "group" && row.modelData.group.collapsed ? "▸" : "▾"; font.pixelSize: 12; opacity: 0.7 }
                    Label {
                        text: row.kind === "group" ? row.modelData.group.name : ""
                        font.weight: Font.DemiBold
                        font.capitalization: Font.AllUppercase
                        font.pixelSize: 11
                        opacity: 0.75
                        Layout.fillWidth: true
                        elide: Text.ElideRight
                    }
                    Label { text: row.kind === "group" ? row.modelData.group.hosts.length : ""; opacity: 0.5; font.pixelSize: 11 }
                }

                // --- host row ---
                RowLayout {
                    visible: row.kind === "host"
                    anchors { fill: parent; leftMargin: 14 + 14 * (row.modelData.indent || 0); rightMargin: 10 }
                    spacing: 8
                    Rectangle {
                        width: 8; height: 8; radius: 4
                        color: row.kind === "host" && row.modelData.host.sessions.some(s => s.live) ? "#4caf50" : "transparent"
                        border.color: Qt.rgba(pal.windowText.r, pal.windowText.g, pal.windowText.b, 0.35)
                        border.width: color == "#00000000" ? 1 : 0
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0
                        Label {
                            text: row.kind === "host" ? row.modelData.host.name : ""
                            font.weight: Font.Medium
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                        Label {
                            text: row.kind === "host" ? row.modelData.host.subtitle : ""
                            font.pixelSize: 11
                            opacity: 0.6
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }
                    }
                }

                // --- session row ---
                RowLayout {
                    visible: row.kind === "session"
                    anchors { fill: parent; leftMargin: 14 + 14 * (row.modelData.indent || 0) + 8; rightMargin: 10 }
                    spacing: 8
                    Rectangle {
                        width: 7; height: 7; radius: 3.5
                        color: {
                            if (row.kind !== "session") return "transparent"
                            switch (row.modelData.session.state) {
                            case "connected": return "#4caf50"
                            case "connecting": return "#e0a030"
                            case "failed": return "#d05050"
                            default: return "#888888"
                            }
                        }
                    }
                    Label {
                        text: row.kind === "session" ? row.modelData.session.label : ""
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                        font.pixelSize: 12
                    }
                    Rectangle {
                        visible: row.kind === "session" && (row.modelData.session.agent || row.modelData.session.formerAgent)
                        radius: 3
                        color: "#2d6a8a"
                        implicitWidth: badge.implicitWidth + 8
                        implicitHeight: badge.implicitHeight + 2
                        Label {
                            id: badge
                            anchors.centerIn: parent
                            color: "white"
                            font.pixelSize: 9
                            font.weight: Font.DemiBold
                            text: row.kind !== "session" ? "" : row.modelData.session.agent
                                  ? (row.modelData.session.agentPaused ? "AGENT · PAUSED" : "AGENT") : "AGENT · LEFT"
                        }
                    }
                }

                TapHandler {
                    acceptedButtons: Qt.LeftButton
                    onTapped: {
                        if (row.kind === "host") win.report(app.openHost(row.modelData.host.id))
                        else if (row.kind === "session") app.selectSession(row.modelData.session.id)
                        else if (row.kind === "group") app.setCollapsed(row.modelData.group.id, !row.modelData.group.collapsed)
                    }
                }
                TapHandler {
                    acceptedButtons: Qt.RightButton
                    onTapped: (point) => {
                        if (row.kind === "host") { root.ctxHost = row.modelData.host; hostMenu.popup() }
                        else if (row.kind === "session") { root.ctxSession = row.modelData.session; root.ctxHost = row.modelData.host; sessionMenu.popup() }
                        else { root.ctxGroup = row.modelData.group; groupMenu.popup() }
                    }
                }
            }
        }
        }

        Rectangle { Layout.fillWidth: true; height: 1; color: Qt.darker(pal.window, 1.2) }
        RowLayout {
            Layout.fillWidth: true
            Layout.margins: 6
            spacing: 2
            ToolButton { text: "+"; font.pixelSize: 18; onClicked: hostEditor.openNew(); ToolTip.text: "Add host (Ctrl+N)"; ToolTip.visible: hovered; ToolTip.delay: 600 }
            ToolButton {
                text: "▤"
                ToolTip.text: "Add group"; ToolTip.visible: hovered; ToolTip.delay: 600
                onClicked: {
                    const id = app.addGroup()
                    if (id.length > 0) textPrompt.ask("Name the group", "New Group", function (v) { win.report(app.renameGroup(id, v)) })
                }
            }
            Item { Layout.fillWidth: true }
            // Sync state, only once a profile exists; click opens Settings.
            Row {
                visible: settings.sync.profiles.length > 0
                spacing: 6
                Layout.alignment: Qt.AlignVCenter
                Rectangle { width: 8; height: 8; radius: 4; anchors.verticalCenter: parent.verticalCenter; color: settings.dotColor(settings.sync.status) }
                Label { text: settings.sync.statusLabel; font.pixelSize: 11; opacity: 0.7; elide: Text.ElideRight; width: Math.min(implicitWidth, 120) }
                TapHandler { onTapped: settings.openSettings(0) }
            }
            ToolButton { text: "⋯"; font.pixelSize: 18; onClicked: mainMenu.popup(); ToolTip.text: "Menu"; ToolTip.visible: hovered; ToolTip.delay: 600 }
        }
    }

    Menu {
        id: mainMenu
        MenuItem { text: "Import from ~/.ssh/config…"; onTriggered: importDialog.openDialog() }
        MenuItem { text: "Settings…"; onTriggered: settings.openSettings(0) }
        MenuItem { text: "Agent access…"; onTriggered: settings.openSettings(1) }
        MenuItem { text: "Lock vault"; onTriggered: app.lock() }
        MenuSeparator {}
        MenuItem { text: "Quit"; onTriggered: { win.quitting = true; app.quit(); Qt.quit() } }
    }

    Menu {
        id: hostMenu
        MenuItem { text: "Open"; onTriggered: win.report(app.openHost(root.ctxHost.id)) }
        MenuItem { text: "New session"; onTriggered: win.report(app.newSession(root.ctxHost.id)) }
        MenuSeparator {}
        MenuItem { text: "Edit…"; onTriggered: hostEditor.openEdit(root.ctxHost.id) }
        MenuItem { text: "Duplicate"; onTriggered: win.report(app.duplicateHost(root.ctxHost.id)) }
        MenuItem { text: "Copy credentials"; onTriggered: clipboard.copy(app.credentials(root.ctxHost.id)) }
        Menu {
            id: moveMenu
            title: "Move to group"
            Instantiator {
                model: [{ id: "", name: "No group" }].concat(win.snap.groups)
                delegate: MenuItem {
                    required property var modelData
                    text: modelData.name
                    onTriggered: app.moveHost(root.ctxHost.id, modelData.id, "")
                }
                onObjectAdded: (index, object) => moveMenu.insertItem(index, object)
                onObjectRemoved: (index, object) => moveMenu.removeItem(object)
            }
        }
        MenuSeparator {}
        MenuItem {
            text: "Delete…"
            onTriggered: confirmDialog.ask("Delete host", "Delete “" + root.ctxHost.name + "”, its saved password and its sessions?", "Delete",
                                           function () { win.report(app.deleteHost(root.ctxHost.id)) })
        }
    }

    Menu {
        id: sessionMenu
        MenuItem { text: "Rename…"; onTriggered: textPrompt.ask("Rename session", root.ctxSession.label, function (v) { win.report(app.renameSession(root.ctxSession.id, v)) }) }
        MenuItem { text: "Reconnect"; onTriggered: app.reconnectSession(root.ctxSession.id) }
        MenuItem { text: "Disconnect"; enabled: root.ctxSession !== null && root.ctxSession.live; onTriggered: app.disconnectSession(root.ctxSession.id) }
        MenuSeparator {}
        Menu {
            id: handMenu
            title: "Hand to agent"
            enabled: settings.agents.ready === true && settings.agents.enabled === true && settings.agents.clients.length > 0
                     && root.ctxSession !== null && !root.ctxSession.agent
            Instantiator {
                model: settings.agents.clients ? settings.agents.clients : []
                delegate: MenuItem {
                    required property var modelData
                    text: modelData.name
                    onTriggered: win.report(app.agentHandOff(root.ctxSession.id, modelData.id))
                }
                onObjectAdded: (index, object) => handMenu.insertItem(index, object)
                onObjectRemoved: (index, object) => handMenu.removeItem(object)
            }
        }
        MenuItem { text: "Stop agent"; enabled: root.ctxSession !== null && !!root.ctxSession.agent; onTriggered: app.agentStop(root.ctxSession.id) }
        MenuItem { text: "Keep session"; enabled: root.ctxSession !== null && root.ctxSession.ephemeral === true; onTriggered: app.agentKeep(root.ctxSession.id) }
        MenuSeparator {}
        MenuItem { text: "Close session"; onTriggered: win.report(app.closeSession(root.ctxSession.id)) }
    }

    Menu {
        id: groupMenu
        MenuItem { text: "Rename…"; onTriggered: textPrompt.ask("Rename group", root.ctxGroup.name, function (v) { win.report(app.renameGroup(root.ctxGroup.id, v)) }) }
        MenuItem { text: "Delete group"; onTriggered: app.deleteGroup(root.ctxGroup.id) }
    }
}
