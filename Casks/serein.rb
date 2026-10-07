cask "serein" do
  version "1.0.0-nightly.20261004.54"
  sha256 "741ae1d13e56199787e3bc63495cf9921cb29521091d61361bea545362979889"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
