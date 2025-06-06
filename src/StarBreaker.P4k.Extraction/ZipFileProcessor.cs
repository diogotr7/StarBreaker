﻿using System.IO.Compression;

namespace StarBreaker.P4k.Extraction;

public sealed class ZipFileProcessor : IFileProcessor
{
    public bool CanProcess(string entryName, Stream stream)
    {
        //todo: optimistic list of extensions?
        return entryName.EndsWith(".zip", StringComparison.OrdinalIgnoreCase);
    }

    public void ProcessEntry(string outputRootFolder, string entryName, Stream entryStream)
    {
        var entryPath = Path.Combine(outputRootFolder, Path.ChangeExtension(entryName, "unzipped"));

        using var archive = new ZipArchive(entryStream, ZipArchiveMode.Read, leaveOpen: true);
        foreach (var childEntry in archive.Entries)
        {
            if (childEntry.Length == 0)
                continue;
            
            using var childStream = childEntry.Open();

            var processor = FileProcessors.GetProcessor(childEntry.FullName, childStream);
            processor.ProcessEntry(entryPath, childEntry.FullName, childStream);
        }
    }
}