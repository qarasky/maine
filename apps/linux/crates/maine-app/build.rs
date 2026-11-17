use cxx_qt_build::{CxxQtBuilder, QmlModule};

const QML: &[&str] = &[
    "qml/Main.qml",
    "qml/Sidebar.qml",
    "qml/SessionHeader.qml",
    "qml/EmptyState.qml",
    "qml/UnlockGate.qml",
    "qml/HostEditor.qml",
    "qml/TextPrompt.qml",
    "qml/ConfirmDialog.qml",
    "qml/ImportDialog.qml",
    "qml/AskpassDialog.qml",
    "qml/SettingsDialog.qml",
    "qml/DemoDriver.qml",
];

fn main() {
    println!("cargo:rerun-if-changed=cpp");
    println!("cargo:rerun-if-changed=qml");
    CxxQtBuilder::new_qml_module(QmlModule::new("app.maine.ui").qml_files(QML))
        .qt_module("Quick")
        .qt_module("Qml")
        .files(["src/bridge.rs"])
        .cpp_files(["cpp/terminal_item.h", "cpp/terminal_item.cpp"])
        .build();
}
