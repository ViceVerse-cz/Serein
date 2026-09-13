cask "serein" do
  version "1.0.0-nightly.9.1"
  sha256 "2d3105bd09e6d7766204b72b98a138e9282dca0fcd96eb703ba3091a7399d2e4"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
