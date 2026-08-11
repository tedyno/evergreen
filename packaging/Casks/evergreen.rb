cask "evergreen" do
  version "0.1.0"
  # Replace with the real values when you cut a release:
  #   shasum -a 256 Evergreen-<version>.zip
  sha256 :no_check

  url "https://github.com/tedyno/evergreen/releases/download/v#{version}/Evergreen-#{version}.zip"
  name "Evergreen"
  desc "Self-hosted sideloading that keeps sideloaded iOS apps alive"
  homepage "https://github.com/tedyno/evergreen"

  depends_on macos: ">= :ventura"

  app "Evergreen.app"

  # Evergreen installs a per-user LaunchAgent (com.evergreen.server) so the engine and
  # its refresh scheduler keep running when the app is closed. Tear it down on uninstall.
  uninstall quit:      "com.evergreen.app",
            launchctl: "com.evergreen.server",
            delete:    "~/Library/LaunchAgents/com.evergreen.server.plist"

  # `brew uninstall --zap` also removes the server's data and preferences.
  zap trash: [
    "~/Library/Application Support/evergreen",
    "~/Library/Preferences/com.evergreen.app.plist",
    "~/Library/LaunchAgents/com.evergreen.server.plist",
  ]
end
