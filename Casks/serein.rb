cask "serein" do
  version "1.0.0-nightly.20260927.48"
  sha256 "c0d462fca08e81f69d103826ece8432d61901101a8d5b7de1ae0716c8e04c1cc"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
