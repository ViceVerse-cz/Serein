cask "serein" do
  version "1.0.0-nightly.20260919.40"
  sha256 "311d10c44c3acb55978462649b634f1bf7fb5a55d9d8599cfea7ee06f6c606ba"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
