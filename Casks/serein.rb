cask "serein" do
  version "1.0.0-nightly.20260928.49"
  sha256 "157532415a8ef531806fb534f3c55640af059b6ddf178733d45fd0f55a3b3a45"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
