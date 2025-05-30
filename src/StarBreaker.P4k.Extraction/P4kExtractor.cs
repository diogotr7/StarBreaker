using System.IO.Enumeration;
using System.Text.RegularExpressions;

namespace StarBreaker.P4k.Extraction;

public enum DdsExtractMode
{
    None,
    Combine,
    ConvertToPng,
}

public class P4kExtractOptions
{
    public required bool ConvertCryXml { get; init; }
    public required bool ExtractSocPak { get; init; }
    public required DdsExtractMode ConvertDds { get; init; }

    //TODO: models (cgf)
    //TODO: sounds (wwise)
    //TODO: videos (bik)
    //TODO: ???
}

public sealed class P4kExtractor
{
    private readonly P4kFile _p4KFile;

    public P4kExtractor(P4kFile p4KFile)
    {
        _p4KFile = p4KFile;
    }

    public void ExtractFiltered(string outputDir, string? filter = null, IProgress<double>? progress = null, bool forceSequential = false)
    {
        //TODO: if the filter is for *.dds, make sure to include *.dds.N too. Maybe do the pre processing before we filter?
        var filteredEntries = (filter is null
            ? _p4KFile.Entries.ToArray()
            : _p4KFile.Entries.Where(entry => FileSystemName.MatchesSimpleExpression(filter, entry.Name))).ToArray();

        Array.Sort(filteredEntries, (a, b) => a.Offset.CompareTo(b.Offset));

        if (forceSequential)
            ExtractEntriesSequential(outputDir, filteredEntries, progress);
        else
            ExtractEntriesParallel(outputDir, filteredEntries, progress);
    }

    public void ExtractRegex(string outputDir, string? regex = null, IProgress<double>? progress = null, bool forceSequential = false)
    {
        var filteredEntries = (regex is null
            ? _p4KFile.Entries.ToArray()
            : _p4KFile.Entries.Where(entry => Regex.IsMatch(entry.Name, regex))).ToArray();

        Array.Sort(filteredEntries, (a, b) => a.Offset.CompareTo(b.Offset));

        if (forceSequential)
            ExtractEntriesSequential(outputDir, filteredEntries, progress);
        else
            ExtractEntriesParallel(outputDir, filteredEntries, progress);
    }

    public void ExtractNode(string outputDir, P4kDirectoryNode directoryNode, IProgress<double>? progress = null)
    {
        var entries = directoryNode.CollectEntries().ToList();

        ExtractEntriesParallel(outputDir, entries, progress);
    }

    public void ExtractEntriesSequential(string outputDir, ICollection<P4kEntry> entries, IProgress<double>? progress = null)
    {
        var numberOfEntries = entries.Count;
        var fivePercent = Math.Max(numberOfEntries / 20, 1);
        var processedEntries = 0;

        progress?.Report(0);

        foreach (var entry in entries)
        {
            ExtractEntry(outputDir, entry);

            processedEntries++;
            if (processedEntries == numberOfEntries || processedEntries % fivePercent == 0)
            {
                progress?.Report(processedEntries / (double)numberOfEntries);
            }
        }

        progress?.Report(1);
    }

    public void ExtractEntriesParallel(string outputDir, ICollection<P4kEntry> entries, IProgress<double>? progress = null)
    {
        var numberOfEntries = entries.Count;
        var fivePercent = Math.Max(numberOfEntries / 20, 1);
        var processedEntries = 0;

        progress?.Report(0);

        var lockObject = new Lock();

        //TODO: Preprocessing step:
        // 1. start with the list of total files
        // 2. run the following according to the filter:
        // 3. find one-shot single file processors
        // 4. find file -> multiple file processors
        // 5. find multiple file -> single file unsplit processors - remove from the list so we don't double process
        // run it!
        Parallel.ForEach(entries, entry =>
            {
                ExtractEntry(outputDir, entry);

                var newIncrement = Interlocked.Increment(ref processedEntries);
                if (newIncrement == numberOfEntries || newIncrement % fivePercent == 0)
                {
                    using (lockObject.EnterScope())
                    {
                        progress?.Report(newIncrement / (double)numberOfEntries);
                    }
                }
            }
        );

        progress?.Report(1);
    }

    public void ExtractEntry(string outputDir, P4kEntry entry)
    {
        if (entry.UncompressedSize == 0)
            return;

        var entryPath = Path.Combine(outputDir, entry.RelativeOutputPath);
        //if (File.Exists(entryPath))
        //    return;

        Directory.CreateDirectory(Path.GetDirectoryName(entryPath) ?? throw new InvalidOperationException());

        using var writeStream = new FileStream(entryPath, FileMode.Create, FileAccess.Write, FileShare.None,
            bufferSize: entry.UncompressedSize > int.MaxValue ? 81920 : (int)entry.UncompressedSize);
        using var entryStream = _p4KFile.OpenStream(entry);

        entryStream.CopyTo(writeStream);
    }
}