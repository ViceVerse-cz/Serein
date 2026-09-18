cask "serein" do
  version "1.0.0-nightly.20260918.39"
  sha256 "0f4a10dc4194c62fd43a9dffcab6e755d57b6552aa942f9161dd6c1dc181b153"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
