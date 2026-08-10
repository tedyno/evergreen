import SwiftUI

@main
struct HomesignStoreApp: App {
    @StateObject private var api = API()
    var body: some Scene {
        WindowGroup {
            RootView().environmentObject(api)
        }
    }
}

struct RootView: View {
    var body: some View {
        TabView {
            CatalogView()
                .tabItem { Label("Aplikace", systemImage: "square.grid.2x2") }
            InstalledView()
                .tabItem { Label("Nainstalované", systemImage: "checkmark.seal") }
            SettingsView()
                .tabItem { Label("Nastavení", systemImage: "gear") }
        }
    }
}
