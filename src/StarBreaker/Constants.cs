using Avalonia.Platform.Storage;

namespace StarBreaker;

public static class Constants
{
    public const string DefaultStarCitizenFolder = @"\Roberts Space Industries\StarCitizen\";
    public const string DataP4k = "Data.p4k";
    public const string BuildManifest = "build_manifest.id";
    public const string DefaultRSILauncherFolder = @"\rsilauncher\logs";

    public static FilePickerOpenOptions GetP4kFilter(IStorageFolder? defaultPath) => new()
    {
        FileTypeFilter =
        [
            new FilePickerFileType("P4k File")
            {
                Patterns = ["*.p4k"],
            }
        ],
        AllowMultiple = false,
        Title = "Select a P4k file",
        SuggestedFileName = DataP4k,
        SuggestedStartLocation = defaultPath
    };
}