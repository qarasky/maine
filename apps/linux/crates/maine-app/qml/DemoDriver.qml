import QtQuick

/// Developer hook (`maine --demo`): creates a vault with a few hosts and opens
/// sessions so the UI can be inspected without typing. Never loaded otherwise.
Item {
    function host(name, hostname, user, groupId) {
        return JSON.stringify({ id: "", name: name, hostname: hostname, port: 22, username: user, password: "", groupId: groupId })
    }
    Component.onCompleted: {
        if (app.state === 0) app.createVault("demo-master-password-1")
        else app.unlock("demo-master-password-1")
        if (win.snap.hostCount === 0) {
            const g = app.addGroup()
            app.renameGroup(g, "Production")
            app.saveHost(host("Web server", "web.example.com", "deploy", null))
            app.saveHost(host("Database", "db.example.com", "postgres", g))
            app.saveHost(host("Build box", "build.example.com", "ci", g))
            app.saveHost(host("Raspberry Pi", "pi.local", "pi", null))
        }
        app.refresh()
        const hosts = win.allHosts()
        if (hosts.length > 0) { app.openHost(hosts[0].id); app.newSession(hosts[0].id) }
        if (hosts.length > 1) app.openHost(hosts[1].id)
        app.refresh()
        const args = Qt.application.arguments
        if (args.indexOf("--demo-password") >= 0) {
            app.saveHost(JSON.stringify({ id: "11111111-2222-3333-4444-555555555555", name: "Pw host", hostname: "pw.example.com",
                                          port: 22, username: "me", password: "s3cret-demo", groupId: null }))
            console.log("HOSTID:11111111-2222-3333-4444-555555555555")
        }
        if (args.indexOf("--demo-editor") >= 0) hostEditor.openEdit(win.allHosts()[0].id)
        for (let i = 0; i < args.length; ++i) {
            if (args[i].indexOf("--demo-sync-create=") === 0) { win.demoOp = "create"; app.syncCreate(args[i].substring(19), "e2e", "") }
            if (args[i].indexOf("--demo-sync-join=") === 0) {
                const parts = args[i].substring(17).split(",")
                win.demoOp = "join"; app.syncJoin(parts[0], parts[1], "e2e", "demo-master-password-1")
            }
        }
        if (args.indexOf("--demo-agent-setup") >= 0) {
            app.agentSetEnabled(true)
            app.agentSetAccess(JSON.stringify({ groups: [], hosts: win.allHosts().map(h => h.id) }))
            app.agentAddClient("e2e-agent")
        }
        if (args.indexOf("--demo-agents") >= 0) { app.agentSetEnabled(true); settings.openSettings(1) }
        if (args.indexOf("--demo-settings") >= 0) settings.openSettings(0)
        if (args.indexOf("--demo-prompt") >= 0)
            askpassDialog.open({ id: 99, host: "Web server", prompt: "Enter passphrase for key '/home/me/.ssh/id_ed25519':", confirmation: false })
    }
}
