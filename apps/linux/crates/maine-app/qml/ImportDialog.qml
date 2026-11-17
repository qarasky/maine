import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

/// Offers the single-host entries of ~/.ssh/config that are not saved yet.
Dialog {
    id: dlg
    modal: true
    anchors.centerIn: Overlay.overlay
    width: Math.min(460, win.width - 40)
    height: Math.min(420, win.height - 40)
    title: "Import from ~/.ssh/config"
    property var candidates: []
    property var checked: ({})

    function openDialog() {
        candidates = JSON.parse(app.importCandidates())
        const c = {}
        for (let i = 0; i < candidates.length; ++i) c[i] = true
        checked = c
        open()
    }

    footer: DialogButtonBox {
        Button {
            text: "Import"
            highlighted: true
            enabled: dlg.candidates.length > 0
            DialogButtonBox.buttonRole: DialogButtonBox.AcceptRole
        }
        Button { text: "Cancel"; DialogButtonBox.buttonRole: DialogButtonBox.RejectRole }
    }

    onAccepted: {
        const chosen = []
        for (let i = 0; i < candidates.length; ++i) if (checked[i]) chosen.push(candidates[i])
        if (chosen.length > 0) win.report(app.importHosts(JSON.stringify(chosen)))
    }

    ColumnLayout {
        anchors.fill: parent
        Label {
            visible: dlg.candidates.length === 0
            text: "Nothing new to import."
            opacity: 0.7
        }
        ListView {
            visible: dlg.candidates.length > 0
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: dlg.candidates
            delegate: CheckDelegate {
                required property var modelData
                required property int index
                width: ListView.view.width
                text: modelData.name + (modelData.hostname !== modelData.name ? "  (" + modelData.hostname + ")" : "")
                checked: dlg.checked[index] === true
                onToggled: { const c = dlg.checked; c[index] = checked; dlg.checked = c }
            }
        }
    }
}
