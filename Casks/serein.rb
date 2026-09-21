cask "serein" do
  version "1.0.0-nightly.20260921.42"
  sha256 "1113754becf7fa037b99e4d382f29360c2caf85f4a7efc74a884c3cb9ace7e38"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
