cask "serein" do
  version "1.0.0-nightly.20261010.57"
  sha256 "f29eedb7d3e43d355b6cba0e96ed958a1578766ed0d9b993b836c06040e1e9b0"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
