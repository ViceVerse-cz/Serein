cask "serein" do
  version "1.0.0-nightly.20261001.53"
  sha256 "cf61e5554156b7ee04c5417580fe99f3106911b3b1b22b4641306a72a5592f0e"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
