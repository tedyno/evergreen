cask "evergreen" do
  version "0.1.0"
  sha256 "ca898a5a127bbd3e06fce95c1129e4afd8541da132608084bfe7e2c2d9c55029"

  url "https://github.com/tedyno/evergreen/releases/download/v#{version}/Evergreen-#{version}.zip"
  name "Evergreen"
  desc "Self-hosted sideloading that keeps sideloaded iOS apps alive"
  homepage "https://github.com/tedyno/evergreen"

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
