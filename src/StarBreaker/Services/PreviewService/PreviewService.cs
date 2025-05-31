using System.Text;
using Avalonia.Media.Imaging;
using Microsoft.Extensions.Logging;
using StarBreaker.Common;
using StarBreaker.Dds;
using StarBreaker.Extensions;
using StarBreaker.P4k;
using StarBreaker.Screens;
using TextMateSharp.Internal.Rules;

namespace StarBreaker.Services;

public interface IPreviewService
{
    FilePreviewViewModel GetPreview(P4kFileNode clickedNode);
}

public class PreviewService : IPreviewService
{
    private readonly ILogger<PreviewService> _logger;
    private readonly IP4kService _p4KService;

    private static readonly string[] plaintextExtensions = [".cfg", ".xml", ".txt", ".json", "eco"];
    private static readonly string[] ddsLodExtensions = [".dds"];
    private static readonly string[] bitmapExtensions = [".bmp", ".jpg", ".jpeg", ".png"];
    //, ".dds.1", ".dds.2", ".dds.3", ".dds.4", ".dds.5", ".dds.6", ".dds.7", ".dds.8", ".dds.9"];

    public PreviewService(IP4kService p4kService, ILogger<PreviewService> logger)
    {
        _p4KService = p4kService;
        _logger = logger;
    }

    public FilePreviewViewModel GetPreview(P4kFileNode selectedEntry)
    {
        //TODO: move this to a service?
        using var entryStream = selectedEntry.P4k.OpenStream(selectedEntry.P4KEntry);

        FilePreviewViewModel preview;

        //check cryxml before extension since ".xml" sometimes is cxml sometimes plaintext
        if (CryXmlB.CryXml.IsCryXmlB(entryStream))
        {
            if (!CryXmlB.CryXml.TryOpen(entryStream, out var c))
            {
                //should never happen
                _logger.LogError("Failed to open CryXmlB");
                return new TextPreviewViewModel("Failed to open CryXmlB");
            }

            _logger.LogInformation("cryxml");
            preview = new TextPreviewViewModel(c.ToString());
        }
        else if (plaintextExtensions.Any(p => selectedEntry.GetName().EndsWith(p, StringComparison.InvariantCultureIgnoreCase)))
        {
            _logger.LogInformation("plaintextExtensions");

            preview = new TextPreviewViewModel(entryStream.ReadString());
        }
        else if (ddsLodExtensions.Any(p => selectedEntry.GetName().EndsWith(p, StringComparison.InvariantCultureIgnoreCase)))
        {
            var ms = DdsFile.MergeToStream(selectedEntry.P4KEntry.Name, selectedEntry.Root.RootNode);
            var pngBytes = DdsFile.ConvertToPng(ms.ToArray());
            _logger.LogInformation("ddsLodExtensions");
            preview = new DdsPreviewViewModel(new Bitmap(pngBytes));
        }
        else if (bitmapExtensions.Any(p => selectedEntry.GetName().EndsWith(p, StringComparison.InvariantCultureIgnoreCase)))
        {
            _logger.LogInformation("bitmapExtensions");
            preview = new DdsPreviewViewModel(new Bitmap(entryStream));
        }
        else
        {
            _logger.LogInformation("hex");
            preview = new HexPreviewViewModel(entryStream.ToArray());
        }
        //todo other types

        return preview;
    }
}