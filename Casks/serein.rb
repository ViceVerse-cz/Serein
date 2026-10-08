cask "serein" do
  version "1.0.0-nightly.20261007.55"
  sha256 "e0f3832787121c652e93875801e1286a9dc3d529e111b1b94d16807f695c933e"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
