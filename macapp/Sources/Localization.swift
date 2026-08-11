import Foundation

/// App-wide localization usable outside SwiftUI views and AppState (e.g. ApiClient,
/// ServerController, PairService). Reads the same "appLanguage" preference AppState writes,
/// so it stays in sync with the in-app language switch.
enum L {
    static func t(_ cs: String, _ en: String) -> String {
        let pref = UserDefaults.standard.string(forKey: "appLanguage") ?? "system"
        let lang: String
        if pref == "cs" || pref == "en" {
            lang = pref
        } else {
            lang = (Locale.preferredLanguages.first ?? "en").hasPrefix("cs") ? "cs" : "en"
        }
        return lang == "cs" ? cs : en
    }
}
