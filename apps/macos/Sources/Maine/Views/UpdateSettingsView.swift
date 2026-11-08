import SwiftUI

/// Settings → Updates: the running version, the automatic-check toggle and a
/// manual check with its outcome. The check is the app's only automatic
/// network call besides sync.
struct UpdateSettingsView: View {
    @ObservedObject private var coordinator = UpdateCoordinator.shared

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Updates")
                .font(.system(size: 15, weight: .semibold))
            Text("Maine asks GitHub Releases for a newer version. No identifiers, accounts or telemetry are involved.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            HStack(spacing: 6) {
                Text("Version")
                    .foregroundStyle(.secondary)
                Text(coordinator.current.description)
                    .monospacedDigit()
            }
            .font(.callout)

            Toggle("Check for updates automatically", isOn: $coordinator.automaticChecks)

            Divider()

            HStack(spacing: 8) {
                Button("Check Now") {
                    Task { await coordinator.check(manual: true) }
                }
                .buttonStyle(MotionButtonStyle())
                .disabled(coordinator.phase.isBusy)
                Text(status)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Spacer(minLength: 0)
        }
        .padding(20)
        .frame(width: 560, alignment: .topLeading)
    }

    private var status: String {
        switch coordinator.phase {
        case .checking:
            return "Checking…"
        case .downloading:
            return "Downloading…"
        case .waitingForIdle:
            return "Waiting for agents and sync to finish…"
        case .failed(let message):
            return message
        case .idle:
            guard let date = coordinator.lastChecked else { return "Not checked yet" }
            return "Up to date. Last checked \(date.formatted(date: .abbreviated, time: .shortened))."
        }
    }
}
